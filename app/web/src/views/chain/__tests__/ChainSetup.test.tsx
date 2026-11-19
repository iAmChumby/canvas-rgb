// The header setup, drawn: the introduction, counting the orange lights, the slide, the devices,
// the two checks, the ways out and the done texts (#381, #382).
//
// The routes are answered here the way the hook's own tests answer them, so what is checked is the
// screen against the hook it is built on: the values the owner moves, the runs the daemon is told
// to light, the writes that end the flow, and the words on the glass. The copy strings are the
// design's, written out here so a changed word fails a test.

import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
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
import type { ChainView } from "@/proto/ChainView";
import type { LightView } from "@/proto/LightView";
import type { SetupLight } from "@/proto/SetupLight";
import { ChainSetup } from "@/views/chain/ChainSetup";
import { PART_COLOURS } from "@/views/chain/partColours";
import type { IntroKind } from "@/views/chain/useChainSetup";

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
const ORANGE = PART_COLOURS[0];
const WHITE = "#ffffff";
const WHY = "The app cannot see what is plugged into this ARGB header.";
const ASK = "Look across your whole case. How many orange lights can you see in total, adding up every fan and strip?";
const HINT =
  "One orange light means your devices are in a row: each takes its own LEDs. An orange light on every device means they share one signal and show the same lights: type how many devices.";
const SLIDE_ONE = "Slide until the orange light is on the last LED of the last device along the cable.";
const SLIDE_MANY = "Slide until the orange light is on the last LED of each device.";
const DEVICES_Q = "How many devices are plugged into JARGB 1?";
const CHECK_Q = "Does each device show its own colour?";
const BLIND = "I can’t see inside";

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

interface ShowProps {
  intro?: IntroKind;
  previousEmpty?: string | null;
  more?: boolean;
}

/** Mounts the screen and lets the session's start answer land. Fake timers are on throughout. */
async function show(chain: ChainView = notSetUp(), props: ShowProps = {}) {
  const onClose = vi.fn();
  const view = render(
    <ChainSetup
      chain={chain}
      onClose={onClose}
      intro={props.intro ?? "none"}
      previousEmpty={props.previousEmpty ?? null}
      more={props.more ?? false}
    />,
  );
  await flush();
  return { ...view, onClose };
}

/** Lets pending answers land and moves the clock by `ms`. */
async function flush(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/**
 * Mounts the screen the way the dev server does: React's strict mode runs every effect twice, so
 * the mount that opens the session is replayed. `next.config.ts` turns strict mode on.
 */
async function showStrict(chain: ChainView = notSetUp(), props: ShowProps = {}) {
  const onClose = vi.fn();
  const view = render(
    <StrictMode>
      <ChainSetup
        chain={chain}
        onClose={onClose}
        intro={props.intro ?? "none"}
        previousEmpty={props.previousEmpty ?? null}
        more={props.more ?? false}
      />
    </StrictMode>,
  );
  await flush();
  return { ...view, onClose };
}

const slider = () => screen.getByRole("slider", { name: "LEDs on this header" });
const valueNow = () => Number(slider().getAttribute("aria-valuenow"));

/** Presses a key by its name and lets what it sends settle. */
async function press(name: string | RegExp): Promise<void> {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
    await vi.advanceTimersByTimeAsync(0);
  });
}

/** Types a number into an entry and commits it with a blur, as the owner does. */
async function typeInto(input: HTMLElement, text: string) {
  fireEvent.change(input, { target: { value: text } });
  await act(async () => {
    fireEvent.blur(input);
  });
}

/** Moves the slider up by `n` LEDs with the arrow key. */
function slideBy(n: number) {
  for (let i = 0; i < n; i++) fireEvent.keyDown(slider(), { key: "ArrowRight" });
}

/** Counts the orange lights: types the number on the first screen. */
async function count(n: number) {
  await typeInto(screen.getByRole("textbox", { name: "Orange lights" }), String(n));
}

/** From the first question to the slide step with the slider at `length`. */
async function toSlide(copies: number, length: number) {
  await count(copies);
  await press("Next");
  slideBy(length - 1);
  await flush(700);
}

/** From the first question to the devices step. */
async function toDevices(length: number, devices: number) {
  await toSlide(1, length);
  await press("Next");
  await typeInto(screen.getByRole("textbox", { name: "Devices on JARGB 1" }), String(devices));
}

/** From the first question to the series check. */
async function toCheck(length: number, devices: number) {
  await toDevices(length, devices);
  await press("Next");
}

/** The runs of the newest light request. */
function lastRuns(): SetupLight["runs"] {
  const call = api.chains.light.mock.calls.at(-1) as [string, number, SetupLight] | undefined;
  return call?.[2].runs ?? [];
}

/** The list of devices on the devices step. */
const devicesList = () => screen.getByRole("list", { name: "Devices on JARGB 1" });

/** The colour of every device chip drawn, in order. */
function chipColours(): (string | null)[] {
  return [...document.querySelectorAll("[data-part-chip]")].map((el) => el.getAttribute("data-colour"));
}

/** Everything the screen says, in one string. */
const words = () => document.body.textContent ?? "";

describe("the header setup screens", () => {
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

  // ---- the introduction -----------------------------------------------------------------

  it("the introduction teaches ARGB header, connector and the zone's name, and starts nothing", async () => {
    await show(notSetUp(), { intro: "full" });

    expect(screen.getByText("Tell the app about your lights.")).toBeTruthy();
    expect(
      screen.getByText(
        "Your motherboard has ARGB headers: small plugs for colour-changing lights. Each fan or strip on one is a device. Your manual may call them connectors. This one is called JARGB 1. Look at your case and answer a few questions.",
      ),
    ).toBeTruthy();
    expect(screen.getByText(WHY)).toBeTruthy();
    expect(screen.getByText("Light setup")).toBeTruthy();
    for (const name of ["Start", "Nothing is plugged in here", "Later"]) {
      expect(screen.getByRole("button", { name })).toBeTruthy();
    }
    expect(
      screen.getByText("Later leaves every light as it is. You can set up this header any time from Devices."),
    ).toBeTruthy();
    // The warning sits under the key that causes it, on the introduction itself (addendum item 2).
    expect(screen.getByText("Anything plugged in here stays dark and leaves your scenes.")).toBeTruthy();
    await flush(5000);
    expect(api.chains.start).not.toHaveBeenCalled();
    expect(api.chains.light).not.toHaveBeenCalled();
  });

  it("Start shows a Starting screen, and the first question is not drawn until the start call answers", async () => {
    let answer: (v: { session: string; restore_to: number; raised_to: number }) => void = () => {};
    api.chains.start.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    await show(notSetUp(), { intro: "full" });

    await press("Start");
    expect(screen.getByText("Getting JARGB 1 ready.")).toBeTruthy();
    expect(screen.getByText("The LEDs on it are about to turn on.")).toBeTruthy();
    expect(screen.queryByText(ASK)).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Orange lights" })).toBeNull();

    await act(async () => {
      answer({ session: SESSION, restore_to: 120, raised_to: 300 });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByText(ASK)).toBeTruthy();
    expect(screen.queryByText("Getting JARGB 1 ready.")).toBeNull();
  });

  it("Later on the introduction closes and opens no session", async () => {
    const { onClose } = await show(notSetUp(), { intro: "full" });

    await press("Later");

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(api.chains.start).not.toHaveBeenCalled();
    expect(api.chains.cancel).not.toHaveBeenCalled();
  });

  it("Nothing is plugged in here marks the header empty in one press, and the next header opens", async () => {
    const { onClose } = await show(notSetUp(), { intro: "full", more: true });

    await press("Nothing is plugged in here");

    expect(screen.queryByText(ASK)).toBeNull();
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
    expect(api.chains.empty).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(api.chains.light).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("the last header marked empty says so and waits for Done", async () => {
    const { onClose } = await show(notSetUp(), { intro: "full", more: false });

    await press("Nothing is plugged in here");

    expect(
      screen.getByText("Marked empty. Set it up again from Devices, then add its lights to your scenes."),
    ).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
    await press("Done");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("the card for a later header is short and names the header marked empty before it", async () => {
    await show(notSetUp({ zone_name: "JARGB 2" }), { intro: "next", previousEmpty: "JAF" });

    expect(screen.getByText("Next: JARGB 2.")).toBeTruthy();
    expect(screen.getByText("Is anything plugged into it?")).toBeTruthy();
    expect(
      screen.getByText("JAF is marked empty. Set it up again from Devices, then add its lights to your scenes."),
    ).toBeTruthy();
    expect(screen.queryByText("Tell the app about your lights.")).toBeNull();
    for (const name of ["Start", "Nothing is plugged in here", "Later"]) {
      expect(screen.getByRole("button", { name })).toBeTruthy();
    }
  });

  // ---- counting the orange lights -------------------------------------------------------

  it("the first question lights ONE orange LED and asks how many orange lights can be seen", async () => {
    await show();

    expect(screen.getByText(ASK)).toBeTruthy();
    expect(screen.getByText(WHY)).toBeTruthy();
    expect(screen.getByText(HINT)).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Orange lights" }) as HTMLInputElement).value).toBe("1");
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);
    expect(screen.getByRole("button", { name: "Nothing lit up" })).toBeTruthy();
    expect(screen.getByRole("button", { name: BLIND })).toBeTruthy();
  });

  it("draws the first question under React's strict mode, with exactly one live session", async () => {
    // The dev server runs effects twice. The cleanup of the first run marks the flow finished, so a
    // second `begin` cancelled both sessions and the screen stayed on "Getting ... ready.".
    await showStrict();

    expect(screen.getByText(ASK)).toBeTruthy();
    expect(screen.queryByText("Getting JARGB 1 ready.")).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    // One session was opened and none was thrown away: exactly one is live.
    expect(api.chains.cancel).not.toHaveBeenCalled();
    await flush(60);
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);
  });

  it("a count above 32 is refused with the reason and a way out, and nothing is taken", async () => {
    await show();

    await typeInto(screen.getByRole("textbox", { name: "Orange lights" }), "40");

    expect(screen.getByText("The app counts up to 32. Count again, or choose Later.")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Orange lights" }) as HTMLInputElement).value).toBe("1");
    expect(screen.getByRole("button", { name: "Later" })).toBeTruthy();

    await typeInto(screen.getByRole("textbox", { name: "Orange lights" }), "8");
    expect(screen.queryByText(/The app counts up to 32/)).toBeNull();
  });

  // ---- the slide ------------------------------------------------------------------------

  it("the slide starts at 1 LED with the single-device prompt, and names its slider", async () => {
    const { getByTestId } = await show();
    await press("Next");

    expect(screen.getByText(SLIDE_ONE)).toBeTruthy();
    expect(getByTestId("chain-length").textContent).toBe("1");
    expect(screen.getByText("LED")).toBeTruthy();
    expect(slider().getAttribute("aria-valuemin")).toBe("1");
    expect(slider().getAttribute("aria-valuemax")).toBe("300");
    expect(valueNow()).toBe(1);
    // "One more step and it goes out" claimed what the app cannot know; it is gone.
    expect(words()).not.toMatch(/it goes out/);
  });

  it("the slide prompt for more than one orange light says each device, and the drawing has a white trail and an orange end", async () => {
    await show();
    await toSlide(8, 8);

    expect(screen.getByText(SLIDE_MANY)).toBeTruthy();
    expect(screen.getByText("LEDs")).toBeTruthy();
    expect(lastRuns()).toEqual([
      { start: 0, count: 7, color: WHITE },
      { start: 7, count: 1, color: ORANGE },
    ]);
    const cells = [...document.querySelectorAll('[data-testid="run-drawing"] span')];
    expect(cells).toHaveLength(8);
    expect(cells.map((c) => c.getAttribute("data-end"))).toEqual([
      "false",
      "false",
      "false",
      "false",
      "false",
      "false",
      "false",
      "true",
    ]);
  });

  it("above 48 LEDs the drawing is a solid bar with an orange tick", async () => {
    await show();
    await press("Next");
    for (let i = 0; i < 5; i++) fireEvent.keyDown(slider(), { key: "ArrowRight", shiftKey: true });
    await flush(700);

    expect(valueNow()).toBe(51);
    expect(screen.getByTestId("run-drawing").getAttribute("data-solid")).toBe("true");
  });

  it("the arrow keys move the length by one and Shift by ten", async () => {
    await show();
    await press("Next");

    fireEvent.keyDown(slider(), { key: "ArrowRight" });
    expect(valueNow()).toBe(2);
    fireEvent.keyDown(slider(), { key: "ArrowLeft" });
    expect(valueNow()).toBe(1);
    fireEvent.keyDown(slider(), { key: "ArrowRight", shiftKey: true });
    expect(valueNow()).toBe(11);
    fireEvent.keyDown(slider(), { key: "ArrowLeft", shiftKey: true });
    expect(valueNow()).toBe(1);
    await flush(60);
  });

  it("the slider stops at the maximum and says why", async () => {
    await show();
    await press("Next");

    expect(screen.queryByText("This header takes at most 300 LEDs.")).toBeNull();
    fireEvent.keyDown(slider(), { key: "End" });
    expect(valueNow()).toBe(300);
    expect(screen.getByText("This header takes at most 300 LEDs.")).toBeTruthy();
    fireEvent.keyDown(slider(), { key: "ArrowRight" });
    expect(valueNow()).toBe(300);
    fireEvent.keyDown(slider(), { key: "ArrowLeft" });
    expect(screen.queryByText("This header takes at most 300 LEDs.")).toBeNull();
  });

  it("Some went dark offers one LED fewer, or to keep sliding", async () => {
    await show();
    await toSlide(8, 9);
    expect(screen.queryByText("Some devices have gone dark. Others still show orange.")).toBeNull();

    await press("Some went dark");
    expect(screen.getByText("Some devices have gone dark. Others still show orange.")).toBeTruthy();
    await press("Keep sliding");
    expect(valueNow()).toBe(9);
    expect(screen.queryByText("Some devices have gone dark. Others still show orange.")).toBeNull();

    await press("Some went dark");
    await press("Use 8 LEDs");
    await flush(700);
    expect(valueNow()).toBe(8);
    expect(lastRuns()).toEqual([
      { start: 0, count: 7, color: WHITE },
      { start: 7, count: 1, color: ORANGE },
    ]);
  });

  it("Some went dark says Use 1 LED, not 1 LEDs", async () => {
    await show();
    await toSlide(8, 2);

    await press("Some went dark");
    expect(screen.getByRole("button", { name: "Use 1 LED" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Use 1 LEDs" })).toBeNull();
  });

  it("a series has no exit of its own on the slide, and I can't see inside is still there", async () => {
    await show();
    await toSlide(1, 5);

    // "Some went dark" is a mirror's (devices of different lengths); "It never goes out" is a
    // mirror's too (a hub that wraps or pads). A series on such a hub has no exit of its own.
    expect(screen.queryByRole("button", { name: "Some went dark" })).toBeNull();
    expect(screen.queryByRole("button", { name: "It never goes out" })).toBeNull();
    // The way out of the flow stays where it was: on the counting screen.
    await press("Back");
    expect(screen.getByRole("button", { name: BLIND })).toBeTruthy();
  });

  it("a mirrored header is offered both slide exits", async () => {
    await show();
    await toSlide(8, 5);

    expect(screen.getByRole("button", { name: "Some went dark" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "It never goes out" })).toBeTruthy();
  });

  it("It never goes out asks how many LEDs one device has and uses that length", async () => {
    await show();
    await toSlide(8, 40);

    await press("It never goes out");
    const field = screen.getByRole("textbox", { name: "LEDs on one device" });
    await typeInto(field, "8");
    await press("Use 8 LEDs");
    await flush(700);

    expect(valueNow()).toBe(8);
    expect(lastRuns()).toEqual([
      { start: 0, count: 7, color: WHITE },
      { start: 7, count: 1, color: ORANGE },
    ]);
    expect(screen.queryByRole("textbox", { name: "LEDs on one device" })).toBeNull();
  });

  // ---- the devices ----------------------------------------------------------------------

  it("the devices step asks per device, says what the count makes, and has rows named Device n", async () => {
    await show();
    await toDevices(36, 3);
    await flush(700); // the figures count to their values

    expect(screen.getByText(DEVICES_Q)).toBeTruthy();
    expect(screen.getByText("Count each device, not each LED: 7 fans and 1 strip is 8.")).toBeTruthy();
    expect(screen.getByText("3 devices, 36 LEDs: 12 each.")).toBeTruthy();
    const rows = within(devicesList()).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual([
      "Device 1·12LEDs",
      "Device 2·12LEDs",
      "Device 3·12LEDs",
    ]);
    expect(chipColours()).toEqual(PART_COLOURS.slice(0, 3));
  });

  it("an uneven split says who gets the extra LED, from the numbers", async () => {
    await show();
    await toDevices(37, 5);

    expect(
      screen.getByText("37 LEDs do not split evenly. The first 2 devices get 8, the rest get 7."),
    ).toBeTruthy();
  });

  it("more devices than LEDs is refused with the reason", async () => {
    await show();
    await toDevices(10, 2);

    await typeInto(screen.getByRole("textbox", { name: "Devices on JARGB 1" }), "11");
    expect(screen.getByText("10 LEDs cannot be split into more than 10 devices.")).toBeTruthy();
    expect(within(devicesList()).getAllByRole("listitem")).toHaveLength(2);
  });

  it("devices equal to LEDs is called unusual, and Count again goes back to counting the orange lights", async () => {
    await show();
    await toDevices(8, 8);

    expect(
      screen.getByText(
        "Eight 1-LED devices is unusual. If every device shows the same colours, one per LED, they share one signal. Count again, and count the orange lights on every device together.",
      ),
    ).toBeTruthy();
    // Saving stays possible: Next is still there.
    expect(screen.getByRole("button", { name: "Next" })).toBeTruthy();

    await press("Count again");
    expect(screen.getByText(ASK)).toBeTruthy();
    await flush(60);
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);
  });

  it("the unusual-count line is not shown for two devices of one LED, or for one device", async () => {
    await show();
    await toDevices(2, 2);
    expect(screen.queryByText(/is unusual/)).toBeNull();

    await typeInto(screen.getByRole("textbox", { name: "Devices on JARGB 1" }), "1");
    expect(screen.queryByText(/is unusual/)).toBeNull();
  });

  it("one row per device with the same colour as its light", async () => {
    await show();
    await toDevices(36, 3);
    await flush(60);

    expect(lastRuns().map((run) => run.color)).toEqual(chipColours());
    expect(lastRuns().map((run) => run.count)).toEqual([12, 12, 12]);
  });

  // ---- the checks and the save ----------------------------------------------------------

  it("the series check asks per device and saves with Yes, save these devices", async () => {
    await show();
    await toCheck(36, 3);

    expect(screen.getByText(CHECK_Q)).toBeTruthy();
    expect(screen.getByText("Change a device’s LEDs to move where its colour ends.")).toBeTruthy();
    await flush(60);

    await press("Yes, save these devices");

    expect(api.chains.commit).toHaveBeenCalledTimes(1);
    expect(api.chains.commit.mock.calls[0]?.[2]).toEqual({
      session: SESSION,
      parts: [
        { name: "Fan 1", kind: "fan", leds: 12 },
        { name: "Fan 2", kind: "fan", leds: 12 },
        { name: "Fan 3", kind: "fan", leds: 12 },
      ],
    });
    expect(screen.getByText("Saved. 3 devices, 36 LEDs on JARGB 1.")).toBeTruthy();
    expect(
      screen.getByText(
        "A device hidden behind a radiator can end the orange early. If one looks short, set up again from Devices, then add its lights to your scenes.",
      ),
    ).toBeTruthy();
  });

  it("changing a row's LED count moves the boundary and calls light", async () => {
    await show();
    await toCheck(120, 2);
    await flush(60);

    expect(lastRuns().map((run) => run.count)).toEqual([60, 60]);
    await typeInto(screen.getByRole("textbox", { name: "LEDs in device 1" }), "40");
    await flush(60);

    expect(lastRuns()).toEqual([
      { start: 0, count: 40, color: PART_COLOURS[0] },
      { start: 40, count: 80, color: PART_COLOURS[1] },
    ]);
    await flush(600);
    expect((screen.getByRole("textbox", { name: "LEDs in device 2" }) as HTMLInputElement).value).toBe("80");
  });

  it("the mirror check has no question, lights every LED white and says so, and saves one row with copies", async () => {
    await show();
    await toSlide(8, 8);
    await press("Next");

    // There is no devices step on the way: the check is next.
    expect(screen.queryByText(DEVICES_Q)).toBeNull();
    expect(screen.getByText("The same 8 LEDs show 8 times on JARGB 1.")).toBeTruthy();
    expect(screen.getByText("You can colour them together. You cannot set one on its own.")).toBeTruthy();
    expect(screen.getByText("All 8 LEDs are lit white now.")).toBeTruthy();
    await flush(60);
    expect(lastRuns()).toEqual([{ start: 0, count: 8, color: WHITE }]);
    expect((screen.getByRole("textbox", { name: "Name of the light" }) as HTMLInputElement).value).toBe(
      "Same lights, shown 8 times",
    );
    expect(screen.queryByRole("radio", { name: "Fan" })).toBeNull();
    expect(screen.queryByText(CHECK_Q)).toBeNull();

    await press("Save");

    expect(api.chains.commit.mock.calls[0]?.[2]).toEqual({
      session: SESSION,
      parts: [{ name: "Same lights, shown 8 times", kind: "other", leds: 8, copies: 8 }],
    });
    expect(screen.getByText("Saved. The same 8 LEDs show 8 times on JARGB 1.")).toBeTruthy();
    expect(
      screen.getByText("Devices of different lengths on this header are stored at the length you chose."),
    ).toBeTruthy();
    expect(words()).not.toMatch(/\bfans?\b/i);
  });

  it("the steps row is Length, Count, Check for a series and Length, Check for a mirror", async () => {
    await show();
    const row = () => [...document.querySelectorAll("ol[aria-label='Steps'] li")];
    const labels = () => row().map((li) => li.textContent);
    const lit = () => row().filter((li) => li.getAttribute("data-now") === "true").map((li) => li.textContent);

    expect(labels()).toEqual(["Length", "Count", "Check"]);
    expect(lit()).toEqual(["Length"]);
    await press("Next");
    await press("Next");
    expect(lit()).toEqual(["Count"]);
    await press("Next");
    expect(lit()).toEqual(["Check"]);
    await press("Back");
    await press("Back");
    await press("Back");
    await count(8);
    expect(labels()).toEqual(["Length", "Check"]);
  });

  it("a commit error shows the device's text and Retry resends", async () => {
    api.chains.commit.mockRejectedValueOnce(
      new ApiFailure(500, "internal", "the header could not be resized: no reply from OpenRGB"),
    );
    await show();
    await toCheck(36, 3);

    await press("Yes, save these devices");
    expect(screen.getByText("the header could not be resized: no reply from OpenRGB")).toBeTruthy();
    expect(screen.getByText(CHECK_Q)).toBeTruthy();
    expect(api.chains.commit).toHaveBeenCalledTimes(1);

    api.chains.commit.mockResolvedValueOnce({ lights: [], final_leds: 36 });
    await press("Retry");
    expect(api.chains.commit).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("the header could not be resized: no reply from OpenRGB")).toBeNull();
  });

  it("a stale session on commit ends the flow, and Retry opens a new one", async () => {
    api.chains.commit.mockRejectedValueOnce(
      new ApiFailure(409, "conflict", "the header setup ended before the save"),
    );
    await show();
    await toCheck(36, 3);

    await press("Yes, save these devices");

    // The daemon let the session go: the save cannot be retried on it, so the flow ends.
    expect(screen.getByText("JARGB 1 did not answer.")).toBeTruthy();
    expect(screen.getByText("the header setup ended before the save")).toBeTruthy();
    expect(screen.queryByText(CHECK_Q)).toBeNull();
    expect(api.chains.commit).toHaveBeenCalledTimes(1);
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    expect(api.chains.cancel).not.toHaveBeenCalled();

    await press("Retry");

    // A new session, as a stale light call's Retry opens one.
    expect(api.chains.start).toHaveBeenCalledTimes(2);
    expect(screen.getByText(ASK)).toBeTruthy();
    expect(api.chains.commit).toHaveBeenCalledTimes(1);
  });

  it("Done closes", async () => {
    const { onClose } = await show();
    await toCheck(36, 3);
    await press("Yes, save these devices");

    await press("Done");

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
  });

  // ---- the ways out ---------------------------------------------------------------------

  it("Nothing lit up lists what to check, writes nothing, and Try again lights LED 1 again", async () => {
    await show();
    await flush(60);
    api.chains.light.mockClear();

    await press("Nothing lit up");

    expect(screen.getByText("No light came on.")).toBeTruthy();
    expect(screen.getByText("Check these, then try again.")).toBeTruthy();
    expect(screen.getByText("Is the cable in the ARGB header called JARGB 1?")).toBeTruthy();
    expect(screen.getByText("Is another lighting app running? Close it, such as SignalRGB.")).toBeTruthy();
    expect(words()).not.toMatch(/Is the PC on/);
    expect(screen.getByText("Anything plugged in here stays dark and leaves your scenes.")).toBeTruthy();
    expect(api.chains.empty).not.toHaveBeenCalled();

    await press("Try again");
    expect(screen.getByText(ASK)).toBeTruthy();
    await flush(60);
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);
    expect(api.chains.empty).not.toHaveBeenCalled();
    expect(api.chains.commit).not.toHaveBeenCalled();
    expect(api.chains.cancel).not.toHaveBeenCalled();
  });

  it("Mark this header empty calls the empty route and, with more headers waiting, closes", async () => {
    const { onClose } = await show(notSetUp(), { more: true });
    await press("Nothing lit up");

    await press("Mark this header empty");

    expect(api.chains.empty).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Mark this header empty on the last header says so and waits for Done", async () => {
    const { onClose } = await show();
    await press("Nothing lit up");

    await press("Mark this header empty");

    expect(screen.getByText(/^Marked empty\./)).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("Mark this header empty is sent once when it is pressed twice", async () => {
    let answer: (v: { ok: boolean }) => void = () => {};
    api.chains.empty.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    await show();
    await press("Nothing lit up");

    const key = screen.getByRole("button", { name: "Mark this header empty" });
    await act(async () => {
      // Two presses inside one tick: the key's own guard has not seen a render, so the hook's
      // in-flight guard is what stops the second one.
      fireEvent.click(key);
      fireEvent.click(key);
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.chains.empty).toHaveBeenCalledTimes(1);
    expect(key.getAttribute("aria-busy")).toBe("true");

    await act(async () => {
      answer({ ok: true });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByText(/^Marked empty\./)).toBeTruthy();
  });

  it("a failed Nothing is plugged in here goes back to the introduction with the error", async () => {
    api.chains.empty.mockRejectedValueOnce(new ApiFailure(500, "internal", "the daemon could not write"));
    await show(notSetUp(), { intro: "full" });

    await press("Nothing is plugged in here");

    // The owner was never shown a light, so the checklist's advice would be about nothing.
    expect(screen.getByText("Tell the app about your lights.")).toBeTruthy();
    expect(screen.getByText("the daemon could not write")).toBeTruthy();
    expect(screen.queryByText("No light came on.")).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);

    // Retry sends the empty call again on the session the start opened; it opens no second one.
    api.chains.empty.mockResolvedValueOnce({ ok: true });
    await press("Retry");
    expect(api.chains.empty).toHaveBeenCalledTimes(2);
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    expect(screen.getByText(/^Marked empty\./)).toBeTruthy();
  });

  it("a stale session under the introduction ends the flow, and Retry starts a new one", async () => {
    // The introduction lights nothing, so the daemon's 6 s timeout can end its session while the
    // owner sits on it. The next empty call then answers 409 (review finding 2).
    api.chains.empty.mockRejectedValueOnce(new ApiFailure(500, "internal", "the daemon could not write"));
    await show(notSetUp(), { intro: "full" });

    await press("Nothing is plugged in here");
    expect(screen.getByText("Tell the app about your lights.")).toBeTruthy();
    expect(screen.getByText("the daemon could not write")).toBeTruthy();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    expect(api.chains.empty).toHaveBeenCalledTimes(1);

    // Seven seconds of the introduction: nothing is sent, so the daemon has let the session go.
    await flush(7000);
    api.chains.empty.mockRejectedValueOnce(
      new ApiFailure(409, "conflict", "this chain setup session is not open any more; start it again."),
    );

    await press("Nothing is plugged in here");

    // Not the daemon's words under the introduction again: the flow ends, as a stale commit does.
    expect(screen.getByText("JARGB 1 did not answer.")).toBeTruthy();
    expect(
      screen.getByText("this chain setup session is not open any more; start it again."),
    ).toBeTruthy();
    expect(screen.queryByText("Tell the app about your lights.")).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    // The empty call went to the session the daemon had let go, twice, and never a third time.
    expect(api.chains.empty.mock.calls.map((call) => call[2])).toEqual([
      { session: SESSION },
      { session: SESSION },
    ]);
    await flush(7000);
    expect(api.chains.empty).toHaveBeenCalledTimes(2);

    // Retry opens a new session and lands on the first question: a dead session's empty write is
    // not repeated by itself, so nothing more is sent until the owner asks for it.
    await press("Retry");
    expect(api.chains.start).toHaveBeenCalledTimes(2);
    expect(screen.getByText(ASK)).toBeTruthy();
    expect(api.chains.empty).toHaveBeenCalledTimes(2);
  });

  it("the banner's Retry sends one commit when it is pressed twice", async () => {
    api.chains.commit.mockRejectedValueOnce(new ApiFailure(500, "internal", "the daemon could not write"));
    await show();
    await toCheck(36, 3);

    await press("Yes, save these devices");
    expect(api.chains.commit).toHaveBeenCalledTimes(1);
    expect(screen.getByText("the daemon could not write")).toBeTruthy();

    // The retry's own answer is still out when the second press arrives.
    let answer: (v: { lights: unknown[]; final_leds: number }) => void = () => {};
    api.chains.commit.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );

    const retry = screen.getByRole("button", { name: "Retry" });
    await act(async () => {
      // Two presses inside one act, so no render lands between them: only a ref can tell them
      // apart. The banner goes as the retry starts (the hook clears the error), so its own busy
      // state is for that same tick, not for a spinner anyone sees.
      fireEvent.click(retry);
      fireEvent.click(retry);
      await vi.advanceTimersByTimeAsync(0);
    });

    // One failed save, one retry: the second press sent nothing.
    expect(api.chains.commit).toHaveBeenCalledTimes(2);

    await act(async () => {
      answer({ lights: [], final_leds: 36 });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByText("Saved. 3 devices, 36 LEDs on JARGB 1.")).toBeTruthy();
  });

  it("a failed mark-empty keeps the checklist open with the daemon's words and Retry", async () => {
    api.chains.empty.mockRejectedValueOnce(new ApiFailure(500, "internal", "the daemon could not write"));
    const { onClose } = await show();
    await press("Nothing lit up");

    await press("Mark this header empty");

    expect(screen.getByText("No light came on.")).toBeTruthy();
    expect(screen.getByText("the daemon could not write")).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
    expect(api.chains.cancel).not.toHaveBeenCalled();

    await press("Retry");
    expect(api.chains.empty).toHaveBeenCalledTimes(2);
    expect(screen.getByText(/^Marked empty\./)).toBeTruthy();
  });

  it("I can't see inside says to look from the side or the back, and writes nothing", async () => {
    await show();
    await press(BLIND);

    expect(
      screen.getByText("Open the side panel or look from the back, then try again. Nothing was changed."),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Later" })).toBeTruthy();
    expect(api.chains.empty).not.toHaveBeenCalled();
    expect(api.chains.commit).not.toHaveBeenCalled();
  });

  it("a failed start shows the device's text and Retry opens a new session", async () => {
    api.chains.start.mockRejectedValueOnce(
      new ApiFailure(503, "unavailable", "the header is busy: another setup holds it"),
    );
    const { getByRole, getByText } = await show();

    expect(getByText("JARGB 1 did not answer.")).toBeTruthy();
    expect(getByText("the header is busy: another setup holds it")).toBeTruthy();
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 120, raised_to: 300 });
    await act(async () => {
      fireEvent.click(getByRole("button", { name: "Retry" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.chains.start).toHaveBeenCalledTimes(2);
    expect(getByText(ASK)).toBeTruthy();
  });

  it("Later and Back from the first question both end the session and close", async () => {
    const first = await show();
    await press("Later");
    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(first.onClose).toHaveBeenCalledTimes(1);
    cleanup();

    api.chains.cancel.mockClear();
    const second = await show();
    await press("Back");
    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(second.onClose).toHaveBeenCalledTimes(1);
  });

  it("Back from the later screens keeps what was entered and ends nothing", async () => {
    await show();
    await toCheck(36, 3);
    await act(async () => {
      fireEvent.change(screen.getByRole("textbox", { name: "Name of device 1" }), {
        target: { value: "Top intake" },
      });
    });

    await press("Back");
    expect(screen.getByText(DEVICES_Q)).toBeTruthy();
    expect(within(devicesList()).getAllByRole("listitem")).toHaveLength(3);
    await press("Back");
    expect(screen.getByText(SLIDE_ONE)).toBeTruthy();
    expect(valueNow()).toBe(36);
    await press("Next");
    await press("Next");
    expect((screen.getByRole("textbox", { name: "Name of device 1" }) as HTMLInputElement).value).toBe(
      "Top intake",
    );
    expect(api.chains.cancel).not.toHaveBeenCalled();
  });

  // ---- words and parts ------------------------------------------------------------------

  it("no screen uses chain, socket, part, addressable or layout", async () => {
    const seen: string[] = [];
    const check = (where: string) => {
      seen.push(where);
      expect(words(), where).not.toMatch(/\bchains?\b|\bsockets?\b|\bparts?\b|addressable|layout/i);
      expect(document.body.innerHTML, where).not.toMatch(/layout/i);
    };
    const intro = await show(notSetUp(), { intro: "full" });
    check("intro");
    intro.unmount();
    const next = await show(notSetUp(), { intro: "next", previousEmpty: "JAF" });
    check("next");
    next.unmount();
    await show();
    check("lit");
    await toSlide(8, 9);
    check("slide");
    await press("Some went dark");
    check("dark");
    await press("Keep sliding");
    await press("It never goes out");
    check("never");
    await press("Keep sliding");
    await press("Next");
    check("mirror check");
    await press("Back");
    await press("Back");
    await count(1);
    await press("Next");
    await press("Next");
    check("devices");
    await press("Next");
    check("series check");
    expect(seen).toHaveLength(9);
  });

  it("Save is only on the mirror check, and nothing is ever labelled Revert", async () => {
    await show();
    const labels = () => [
      ...[...document.querySelectorAll("button")].map((b) => (b.textContent ?? "").trim()),
      ...[...document.querySelectorAll("[aria-label]")].map((e) => e.getAttribute("aria-label") ?? ""),
    ];
    const check = (where: string, save: boolean) => {
      expect(labels(), where).not.toContain("Revert");
      if (save) expect(labels(), where).toContain("Save");
      else expect(labels(), where).not.toContain("Save");
    };
    check("lit", false);
    await press("Next");
    check("slide", false);
    await press("Next");
    check("devices", false);
    await press("Next");
    check("series check", false);
    await press("Back");
    await press("Back");
    await press("Back");
    await count(8);
    await press("Next");
    await press("Next");
    check("mirror check", true);
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
    const intro = await show(notSetUp(), { intro: "full" });
    check("intro");
    intro.unmount();
    await show();
    check("lit");
    await press("Next");
    check("slide");
    await press("Next");
    check("devices");
    await press("Next");
    check("series check");
    await press("Back");
    await press("Back");
    await press("Back");
    await count(8);
    await press("Next");
    await press("Next");
    check("mirror check");
  });

  it("the foot has the same three slots on every screen, with each key in its own slot", async () => {
    const slots = () =>
      ["footBack", "footLater", "footPrimary"].map((name) =>
        [...document.querySelectorAll(`footer > div[class*="${name}"]`)].map((d) => d.textContent?.trim() ?? ""),
      );
    await show(notSetUp(), { intro: "full" });
    expect(slots()).toEqual([[""], ["Later"], ["Start"]]);
    await press("Start");
    expect(slots()).toEqual([["Back"], ["Later"], ["Next"]]);
    await press("Next");
    await press("Next");
    await press("Next");
    expect(slots()).toEqual([["Back"], ["Later"], ["Yes, save these devices"]]);
    await press("Yes, save these devices");
    expect(slots()).toEqual([[""], [""], ["Done"]]);
  });
});
