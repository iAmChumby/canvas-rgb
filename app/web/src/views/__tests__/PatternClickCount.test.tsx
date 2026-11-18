import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { api } from "@/lib/endpoints";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// #481 item 6: how many clicks Luke's flow takes, counted through the real page. From New effect to
// a saved fan pattern: a ring of gradient stops that turns. The page is the real one, against the
// demo backend; every click is a pointer press and release on a control the owner can see, and a
// colour is chosen on the picker's square (a press on it), so the count is the owner's, not the
// hook's. The design (`patterns-motion.md` §2) said 7 clicks keeping the seeded colours and 13 with
// three custom ones; the real flow is 7 and 15, and the difference is stated below.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const BOX = 400;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=patterns");
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
  Element.prototype.setPointerCapture = (id: number) => void captured.add(id);
  Element.prototype.releasePointerCapture = (id: number) => void captured.delete(id);
  Element.prototype.hasPointerCapture = (id: number) => captured.has(id);
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
  // Every element is measured at BOX x BOX, so the device's dots, the stops' handles and the
  // picker's square have a box to be pressed in.
  Element.prototype.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: BOX, height: BOX, right: BOX, bottom: BOX, x: 0, y: 0, toJSON() {} }) as DOMRect;
  HTMLCanvasElement.prototype.getContext = (() => null) as unknown as typeof HTMLCanvasElement.prototype.getContext;
  Range.prototype.getClientRects = (() =>
    ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList) as typeof Range.prototype.getClientRects;
  Range.prototype.getBoundingClientRect = (() =>
    ({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) }) as DOMRect) as typeof Range.prototype.getBoundingClientRect;
});

afterEach(cleanup);

/** Every click the owner makes in a run. */
let clicks: string[] = [];

/** One click: a press and a release on the control, then the click that follows them. */
async function click(label: string, element: Element) {
  clicks.push(label);
  await act(async () => {
    fireEvent.pointerDown(element, { pointerId: 1, clientX: 100, clientY: 60 });
    fireEvent.pointerUp(element, { pointerId: 1, clientX: 100, clientY: 60 });
    fireEvent.click(element);
  });
}

/** Mounts the Effects page, as the owner opens it. */
async function mount() {
  await refreshAll();
  render(
    <>
      <NotifyHost />
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>
    </>,
  );
}

/** New effect, Pattern, the strip of 8 (three sticks of DRAM), Ring, Gradient. Five clicks. */
async function toTheGradient() {
  clicks = [];
  await mount();
  await click("New effect", await screen.findByRole("button", { name: "New effect" }));
  const menu = await screen.findByRole("dialog", { name: "New effect" });
  await click("Pattern", within(menu).getByRole("button", { name: "Pattern" }));
  const step = await screen.findByTestId("build-on");
  await click("Build on the strip of 8", step.querySelector('[data-kind="8:strip"]')!);
  const editor = await screen.findByLabelText(/, a pattern$/);
  expect(editor.textContent).toContain("strip of 8");
  await click("Ring", within(editor).getByRole("radio", { name: "Ring" }));
  await waitFor(() => expect(within(editor).getByRole("radio", { name: "Ring" }).getAttribute("aria-checked")).toBe("true"));
  await click("Gradient", within(editor).getByRole("button", { name: "Gradient" }));
  expect(within(editor).getByRole("button", { name: /^Stop 1,/ })).toBeTruthy();
  return editor;
}

/** Spin (its default speed, 0.25 turns a second), then Save under the name the field already holds. */
async function spinAndSave(editor: HTMLElement) {
  // The name field already holds the next free "Pattern n": nothing is typed.
  const name = (screen.getByLabelText("Name") as HTMLInputElement).value;
  await click("Spin", within(editor).getByRole("radio", { name: "Spin" }));
  await click("Save", within(editor).getByRole("button", { name: "Save" }));
  await waitFor(async () => expect((await api.pattern(name)).id).toBe(`pattern:${name}`));
  return name;
}

describe("the click count of the fan flow, from New effect to a saved spinning ring (#481)", () => {
  it("is 7 clicks keeping the seeded colours", async () => {
    const editor = await toTheGradient();
    expect(clicks).toEqual(["New effect", "Pattern", "Build on the strip of 8", "Ring", "Gradient"]);
    const name = await spinAndSave(editor);
    expect(clicks).toHaveLength(7);
    const saved = await api.pattern(name);
    expect(saved.built_on).toEqual({ led_count: 8, arrangement: "ring" });
    expect(saved.motion).toEqual({ type: "spin", turns_per_s: 0.25 });
    expect(saved.frames).toHaveLength(1);
    // The seeded gradient is on the frame: more than one colour, none of them off.
    const colours = new Set(saved.frames[0]!.colors);
    expect(colours.size).toBeGreaterThan(1);
    expect(colours.has("#000000")).toBe(false);
  });

  it("is 15 clicks with a colour of its own for each of the three stops", async () => {
    const editor = await toTheGradient();
    const stopColours = () =>
      [1, 2, 3].map(
        (n) => /#[0-9a-f]{6}/i.exec(within(editor).getByRole("button", { name: new RegExp(`^Stop ${n},`) }).getAttribute("aria-label") ?? "")![0].toLowerCase(),
      );
    const seeded = stopColours();

    /** Opens the selected stop's colour and chooses on the picker's square: two clicks. */
    const colour = async (stop: number, x: number, y: number) => {
      const field = within(editor).getByRole("button", { name: `Stop ${stop} colour` });
      await click(`Stop ${stop}: open its colour`, field);
      const square = await screen.findByRole("slider", { name: `Stop ${stop} colour saturation and brightness` });
      clicks.push(`Stop ${stop}: choose a colour`);
      const before = stopColours()[stop - 1];
      await act(async () => {
        fireEvent.pointerDown(square, { pointerId: 1, clientX: x, clientY: y });
        fireEvent.pointerUp(square, { pointerId: 1, clientX: x, clientY: y });
      });
      // The picker sends a pointer's colour on the next animation frame.
      await waitFor(() => expect(stopColours()[stop - 1]).not.toBe(before));
    };

    // Stop 1 is the one the tool selects when it opens: no click to reach it.
    await colour(1, 120, 120);
    // Stops 2 and 3: press the stop (which also closes the picker), open its colour, choose.
    for (const [stop, x, y] of [
      [2, 40, 40],
      [3, 200, 360],
    ] as const) {
      await click(`Stop ${stop}: select it`, within(editor).getByRole("button", { name: new RegExp(`^Stop ${stop},`) }));
      await colour(stop, x, y);
    }
    const chosen = stopColours();
    for (let i = 0; i < 3; i += 1) expect(chosen[i], `stop ${i + 1} took a colour of its own`).not.toBe(seeded[i]);
    const name = await spinAndSave(editor);

    expect(clicks).toHaveLength(15);
    const saved = await api.pattern(name);
    // The chosen colours reach the frame and the seeded ones are gone. (A stop that stands between two
    // LEDs, or that a later stop covers, blends into its neighbours, so not every stop colour is an
    // exact LED.)
    const onFrame = new Set(saved.frames[0]!.colors);
    expect(chosen.some((colour) => onFrame.has(colour))).toBe(true);
    for (const colour of seeded) expect(onFrame.has(colour), `the seeded ${colour} is gone`).toBe(false);
    expect(saved.motion).toEqual({ type: "spin", turns_per_s: 0.25 });
  });
});
