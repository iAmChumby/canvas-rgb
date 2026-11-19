import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { layoutDots, type Dot } from "@/views/patterns/dotLayout";
import type { Pattern } from "@/proto/Pattern";

// The frame strip (#468): the frames of a pattern, the transition between them, Loop, fps and the
// preview. The daemon is mocked, and the clock is a stubbed rAF plus fake timers: nothing here
// waits for a real frame or a real second. Every assertion reads what the lights are sent (the
// draft's document) or what the owner sees in the strip.

const { api } = vi.hoisted(() => ({
  api: {
    putPatternDraft: vi.fn(),
    deleteDraft: vi.fn(),
    putPattern: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { PatternEditor } from "@/views/patterns/PatternEditor";
import type { DraftStatus } from "@/proto/DraftStatus";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
/** A ring of 12 drawn in a 400 x 400 box: the box the canvas is measured with in these tests. */
const BOX = 400;

let pending = new Map<number, FrameRequestCallback>();
let nextFrame = 0;

beforeAll(() => {
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
  window.matchMedia = ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
});

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "setInterval", "clearTimeout", "clearInterval", "Date"] });
  pending = new Map();
  nextFrame = 0;
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    nextFrame += 1;
    pending.set(nextFrame, cb);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => void pending.delete(id));
  api.putPatternDraft.mockReset().mockResolvedValue(status());
  api.deleteDraft.mockReset().mockResolvedValue(status({ active: false }));
  api.putPattern.mockReset();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function status(over: Partial<DraftStatus> = {}): DraftStatus {
  return {
    active: true,
    props: {},
    lease_remaining_ms: 15_000,
    heartbeat_ms: 5_000,
    lease_ms: 15_000,
    ...over,
  };
}

/** A ring of 12, `on` naming the LEDs that are not off. */
function ring(on: Record<number, string> = {}): Pattern {
  const colors = Array.from({ length: 12 }, (_, i) => on[i] ?? OFF);
  return {
    id: "pattern:Intake",
    built_on: { led_count: 12, arrangement: "ring" },
    frames: [{ colors, hold: 0 }],
    motion: { type: "none" },
  };
}

/** Renders the editor with a canvas measured at 400 x 400 and the dots that box gives. */
function show(
  pattern: Pattern = ring(),
  props: { note?: string | null; onSave?: (p: Pattern) => void; onCancel?: () => void } = {},
) {
  const view = render(
    <PatternEditor
      pattern={pattern}
      note={props.note}
      onSave={props.onSave ?? vi.fn()}
      onCancel={props.onCancel ?? vi.fn()}
    />,
  );
  const canvas = view.container.querySelector("canvas") as HTMLCanvasElement;
  const rect = { left: 0, top: 0, width: BOX, height: BOX, right: BOX, bottom: BOX, x: 0, y: 0, toJSON() {} } as DOMRect;
  canvas.getBoundingClientRect = () => rect;
  const dots: Dot[] = layoutDots(pattern.built_on, BOX, BOX);
  return { view, canvas, dots };
}

/** Lets the promises of the requests already sent settle. */
const settle = () => act(async () => {});

/** Runs every queued animation frame: the editor's draft goes out on this clock. */
async function frames(): Promise<void> {
  const queued = [...pending.values()];
  pending.clear();
  await act(async () => {
    for (const cb of queued) cb(0);
  });
}


/**
 * Puts Play on, so the lights are sent the whole document (#481): while it is off they get the selected
 * frame alone. These tests read the frames, holds and transitions an edit made, which is the document.
 */
const playAll = async () => {
  fireEvent.click(screen.getByRole("button", { name: "Play preview" }));
  await frames();
};

const at = (dot: Dot) => ({ clientX: dot.x, clientY: dot.y, pointerId: 1 });

const RED = "#ff0000";
const GREEN = "#00ff00";
const BLUE = "#0000ff";

/** A ring of 12 with `colour` on LED 0 for each frame given, every other LED off. */
function framed(colours: string[], holds: number[] = []): Pattern {
  return {
    id: "pattern:Intake",
    built_on: { led_count: 12, arrangement: "ring" },
    frames: colours.map((c, f) => ({
      colors: Array.from({ length: 12 }, (_, i) => (i === 0 ? c : OFF)),
      hold: holds[f] ?? 500,
    })),
    motion: { type: "none" },
  };
}

/** The document the lights were last sent. */
const sent = (): Pattern => api.putPatternDraft.mock.calls.at(-1)![0].pattern;
/** The first LED of each frame the lights were last sent. */
const heads = (): string[] => sent().frames.map((f) => f.colors[0]!);
const holds = (): number[] => sent().frames.map((f) => f.hold);

const key = (n: number) => screen.getByRole("button", { name: new RegExp(`^Frame ${n} of `) });
const keys = () => screen.getAllByRole("button", { name: /^Frame \d+ of / });
const connector = (n: number) => screen.getByRole("button", { name: new RegExp(`^Frame ${n} to ${n + 1}:`) });

describe("the frame strip", () => {
  it("shows a key for every frame, its hold under it, and the first one selected", async () => {
    show(framed([RED, GREEN, BLUE], [100, 250, 1200]));
    await settle();
    expect(keys()).toHaveLength(3);
    expect(key(1).getAttribute("aria-pressed")).toBe("true");
    expect(key(2).getAttribute("aria-pressed")).toBe("false");
    expect(key(2).getAttribute("aria-label")).toBe("Frame 2 of 3, holds 250 milliseconds");
    expect(screen.getByText("100ms")).toBeTruthy();
    expect(screen.getByText("1200ms")).toBeTruthy();
  });

  it("adds a blank frame after the selected one and selects it; undo takes it away", async () => {
    show(framed([RED, GREEN]));
    await settle();
    await playAll();
    fireEvent.click(screen.getByRole("button", { name: "Add frame" }));
    await frames();
    expect(sent().frames).toHaveLength(3);
    expect(heads()).toEqual([RED, OFF, GREEN]);
    expect(key(2).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(heads()).toEqual([RED, GREEN]);
    expect(keys()).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: "Redo" }));
    await frames();
    expect(heads()).toEqual([RED, OFF, GREEN]);
  });

  it("duplicates the selected frame with its hold, and undo is one step", async () => {
    show(framed([RED, GREEN], [100, 700]));
    await settle();
    await playAll();
    fireEvent.click(key(2));
    fireEvent.click(screen.getByRole("button", { name: "Duplicate frame" }));
    await frames();
    expect(heads()).toEqual([RED, GREEN, GREEN]);
    expect(holds()).toEqual([100, 700, 700]);
    expect(key(3).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(heads()).toEqual([RED, GREEN]);
  });

  it("deletes the selected frame, keeps one, and undo brings it back where it was", async () => {
    show(framed([RED, GREEN, BLUE]));
    await settle();
    await playAll();
    expect(screen.getByRole("button", { name: "Delete frame" }).hasAttribute("disabled")).toBe(false);
    fireEvent.click(key(2));
    fireEvent.click(screen.getByRole("button", { name: "Delete frame" }));
    await frames();
    expect(heads()).toEqual([RED, BLUE]);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(heads()).toEqual([RED, GREEN, BLUE]);
    // One frame left: nothing to delete.
    cleanup();
    show(framed([RED]));
    await settle();
    expect(screen.getByRole("button", { name: "Delete frame" }).hasAttribute("disabled")).toBe(true);
  });

  it("reorders with the arrow keys, says where the frame went, and undo puts it back", async () => {
    show(framed([RED, GREEN, BLUE]));
    await settle();
    await playAll();
    key(1).focus();
    fireEvent.keyDown(key(1), { key: "ArrowRight" });
    await frames();
    expect(heads()).toEqual([GREEN, RED, BLUE]);
    expect(document.body.textContent).toContain("Frame 1 moved to position 2 of 3.");
    // The selection followed the frame it was on.
    expect(key(2).getAttribute("aria-pressed")).toBe("true");
    fireEvent.keyDown(key(2), { key: "ArrowRight" });
    await frames();
    expect(heads()).toEqual([GREEN, BLUE, RED]);
    fireEvent.keyDown(key(3), { key: "ArrowRight" });
    expect(document.body.textContent).toContain("Frame 3 is already last.");
    expect(heads()).toEqual([GREEN, BLUE, RED]);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(heads()).toEqual([GREEN, RED, BLUE]);
  });

  it("reorders with the pointer: press, travel past the threshold, drop in a gap", async () => {
    show(framed([RED, GREEN, BLUE]));
    await settle();
    await playAll();
    // Three keys 100 px apart: the box the pointer is measured against.
    keys().forEach((k, i) => {
      k.getBoundingClientRect = () => ({ left: i * 100, right: i * 100 + 60, top: 0, bottom: 48, width: 60, height: 48, x: i * 100, y: 0, toJSON() {} }) as DOMRect;
    });
    fireEvent.pointerDown(key(1), { clientX: 30, clientY: 20, pointerId: 1, pointerType: "mouse", button: 0, buttons: 1 });
    // Under the threshold: a press, not a drag.
    fireEvent.pointerMove(window, { clientX: 32, clientY: 20, pointerId: 1, buttons: 1 });
    expect(screen.queryByTestId("frame-marker")).toBeNull();
    // Past it, and past the middle of frame 3: the marker stands after the last key.
    fireEvent.pointerMove(window, { clientX: 290, clientY: 20, pointerId: 1, buttons: 1 });
    expect(screen.getByTestId("frame-marker")).toBeTruthy();
    fireEvent.pointerUp(window, { pointerId: 1 });
    await frames();
    expect(heads()).toEqual([GREEN, BLUE, RED]);
    // The click that ends the drag is not a choice of a frame.
    fireEvent.click(key(3));
    expect(key(1).getAttribute("aria-pressed")).toBe("false");
  });

  it("Escape calls a drag off and writes nothing", async () => {
    show(framed([RED, GREEN, BLUE]));
    await settle();
    keys().forEach((k, i) => {
      k.getBoundingClientRect = () => ({ left: i * 100, right: i * 100 + 60, top: 0, bottom: 48, width: 60, height: 48, x: i * 100, y: 0, toJSON() {} }) as DOMRect;
    });
    api.putPatternDraft.mockClear();
    fireEvent.pointerDown(key(1), { clientX: 30, clientY: 20, pointerId: 1, pointerType: "mouse", button: 0, buttons: 1 });
    fireEvent.pointerMove(window, { clientX: 290, clientY: 20, pointerId: 1, buttons: 1 });
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.pointerUp(window, { pointerId: 1 });
    await frames();
    expect(screen.queryByTestId("frame-marker")).toBeNull();
    expect(api.putPatternDraft).not.toHaveBeenCalled();
  });

  it("paints the selected frame only", async () => {
    const { canvas, dots } = show(framed([RED, GREEN]));
    await settle();
    await playAll();
    fireEvent.click(key(2));
    fireEvent.pointerDown(canvas, at(dots[3]!));
    fireEvent.pointerUp(canvas, { pointerId: 1 });
    await frames();
    const [first, second] = sent().frames;
    expect(first!.colors.filter((c) => c !== OFF)).toEqual([RED]);
    expect(second!.colors[0]).toBe(GREEN);
    expect(second!.colors[3]).not.toBe(OFF);
    // Painting is one undo step and undo puts the selected frame back, not another one.
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(sent().frames[1]!.colors[3]).toBe(OFF);
    expect(key(2).getAttribute("aria-pressed")).toBe("true");
  });
});

describe("transitions", () => {
  it("Loop set to Fade 800 ms still says Fade 800 ms after a frame is added at the end (#468)", async () => {
    show(framed([RED, GREEN]));
    await settle();
    await playAll();
    fireEvent.click(screen.getByRole("button", { name: /^After frame 2:/ }));
    fireEvent.click(screen.getByRole("radio", { name: "Fade" }));
    const field = screen.getByRole("textbox", { name: "Frame 2 fade duration" });
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "800" } });
    fireEvent.blur(field);
    await frames();
    expect(screen.getByRole("button", { name: "After frame 2: loops to frame 1, fading 800 milliseconds" })).toBeTruthy();
    fireEvent.click(key(2));
    fireEvent.click(screen.getByRole("button", { name: "Add frame" }));
    await frames();
    expect(keys()).toHaveLength(3);
    expect(screen.getByRole("button", { name: "After frame 3: loops to frame 1, fading 800 milliseconds" })).toBeTruthy();
    expect(sent().frames.at(-1)!.transition).toEqual({ kind: "fade", ms: 800, easing: "linear" });
    expect(connector(2).getAttribute("aria-label")).toBe("Frame 2 to 3: cut");
  });

  it("a connector starts as a cut and its panel makes it a fade of 400 ms, linear", async () => {
    show(framed([RED, GREEN]));
    await settle();
    await playAll();
    expect(connector(1).getAttribute("aria-label")).toBe("Frame 1 to 2: cut");
    fireEvent.click(connector(1));
    fireEvent.click(screen.getByRole("radio", { name: "Fade" }));
    await frames();
    expect(sent().frames[0]!.transition).toEqual({ kind: "fade", ms: 400, easing: "linear" });
    expect(sent().frames[1]!.transition).toBeUndefined();
    expect(connector(1).getAttribute("aria-label")).toBe("Frame 1 to 2: fade, 400 milliseconds");
  });

  it("edits the duration and the pace, and undo steps back through them", async () => {
    show(framed([RED, GREEN]));
    await settle();
    await playAll();
    fireEvent.click(connector(1));
    fireEvent.click(screen.getByRole("radio", { name: "Fade" }));
    const field = screen.getByRole("textbox", { name: "Frame 1 fade duration" });
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "750" } });
    fireEvent.blur(field);
    fireEvent.click(screen.getByRole("radio", { name: "Out" }));
    await frames();
    expect(sent().frames[0]!.transition).toEqual({ kind: "fade", ms: 750, easing: "ease_out" });
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(sent().frames[0]!.transition).toEqual({ kind: "fade", ms: 750, easing: "linear" });
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(sent().frames[0]!.transition).toEqual({ kind: "fade", ms: 400, easing: "linear" });
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(sent().frames[0]!.transition).toBeUndefined();
  });

  it("a duration past the spine's cap is held at it", async () => {
    show(framed([RED, GREEN]));
    await settle();
    await playAll();
    fireEvent.click(connector(1));
    fireEvent.click(screen.getByRole("radio", { name: "Fade" }));
    const field = screen.getByRole("textbox", { name: "Frame 1 fade duration" });
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "999999" } });
    fireEvent.blur(field);
    await frames();
    expect(sent().frames[0]!.transition!.ms).toBe(60_000);
  });

  it("the last connector is Loop: off stops on the last frame, on writes no playback at all", async () => {
    show(framed([RED, GREEN]));
    await settle();
    await playAll();
    expect("playback" in sent()).toBe(false);
    const last = screen.getByRole("button", { name: /^After frame 2:/ });
    expect(last.getAttribute("aria-label")).toBe("After frame 2: loops to frame 1, cutting");
    fireEvent.click(last);
    fireEvent.click(screen.getByRole("switch", { name: "Loop back to frame 1" }));
    await frames();
    expect(sent().playback).toEqual({ loop: false });
    expect(screen.getByRole("button", { name: "After frame 2: stops here" })).toBeTruthy();
    fireEvent.click(screen.getByRole("switch", { name: "Loop back to frame 1" }));
    await frames();
    expect("playback" in sent()).toBe(false);
  });

  it("a frame a later version wrote keeps every field it carries through a transition edit", async () => {
    const loaded = framed([RED, GREEN]);
    (loaded.frames[1] as unknown as { label: string }).label = "kept";
    (loaded as unknown as { tags: string[] }).tags = ["kept"];
    show(loaded);
    await settle();
    await playAll();
    fireEvent.click(connector(1));
    fireEvent.click(screen.getByRole("radio", { name: "Fade" }));
    await frames();
    expect((sent().frames[1] as unknown as { label: string }).label).toBe("kept");
    expect((sent() as unknown as { tags: string[] }).tags).toEqual(["kept"]);
  });
});

describe("timing", () => {
  it("edits the selected frame's hold", async () => {
    show(framed([RED, GREEN], [100, 200]));
    await settle();
    await playAll();
    fireEvent.click(key(2));
    const field = screen.getByRole("textbox", { name: "Frame 2 hold" });
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "640" } });
    fireEvent.blur(field);
    await frames();
    expect(holds()).toEqual([100, 640]);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(holds()).toEqual([100, 200]);
  });

  it("fps sets every hold to round(1000 / fps) and every transition to a cut, and stores no fps", async () => {
    const loaded = framed([RED, GREEN, BLUE], [100, 900, 40]);
    loaded.frames[0]!.transition = { kind: "fade", ms: 300, easing: "ease_in" };
    show(loaded);
    await settle();
    await playAll();
    fireEvent.click(screen.getByRole("radio", { name: "fps" }));
    const field = screen.getByRole("textbox", { name: "Frames per second" });
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "12" } });
    fireEvent.blur(field);
    await frames();
    expect(holds()).toEqual([83, 83, 83]);
    expect(sent().frames.every((f) => f.transition === undefined)).toBe(true);
    expect(JSON.stringify(sent())).not.toMatch(/fps/);
    // One history step restores the holds and the fade.
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(holds()).toEqual([100, 900, 40]);
    expect(sent().frames[0]!.transition).toEqual({ kind: "fade", ms: 300, easing: "ease_in" });
  });

  it("shows mixed, not a made-up 10, when the holds differ, and a typed fps still sets them all (#481)", async () => {
    show(framed([RED, GREEN, BLUE], [100, 900, 40]));
    await settle();
    await playAll();
    fireEvent.click(screen.getByRole("radio", { name: "fps" }));
    const field = screen.getByRole("textbox", { name: "Frames per second" }) as HTMLInputElement;
    expect(field.value).toBe("");
    expect(field.placeholder).toBe("mixed");
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "12" } });
    fireEvent.blur(field);
    await frames();
    expect(holds()).toEqual([83, 83, 83]);
  });

  it("opens in fps when the holds are alike and every frame cuts", async () => {
    show(framed([RED, GREEN, BLUE], [100, 100, 100]));
    await settle();
    expect(screen.getByRole("radio", { name: "fps" }).getAttribute("aria-checked")).toBe("true");
    expect((screen.getByRole("textbox", { name: "Frames per second" }) as HTMLInputElement).value).toBe("10");
  });
});

describe("the preview", () => {
  it("Play runs the document: the key is lit while it plays, and Stop ends it", async () => {
    show(framed([RED, GREEN], [200, 200]));
    await settle();
    const play = screen.getByRole("button", { name: "Play preview" });
    expect(play.hasAttribute("disabled")).toBe(false);
    fireEvent.click(play);
    expect(screen.getByRole("button", { name: "Stop preview" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "Stop preview" }));
    expect(screen.getByRole("button", { name: "Play preview" }).getAttribute("aria-pressed")).toBe("false");
  });

  it("has nothing to play for one frame, or for frames that take no time", async () => {
    show(framed([RED]));
    await settle();
    expect(screen.getByRole("button", { name: /^Nothing to play/ }).hasAttribute("disabled")).toBe(true);
    cleanup();
    show(framed([RED, GREEN], [0, 0]));
    await settle();
    expect(screen.getByRole("button", { name: /^Nothing to play/ }).hasAttribute("disabled")).toBe(true);
  });

  it("honours reduced motion: the preview cannot play and says why", async () => {
    window.matchMedia = ((query: string) =>
      ({ matches: query.includes("reduce"), media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
    show(framed([RED, GREEN], [200, 200]));
    await settle();
    const play = screen.getByRole("button", { name: /^Reduced motion is on/ });
    expect(play.hasAttribute("disabled")).toBe(true);
    window.matchMedia = ((query: string) =>
      ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
  });
});

describe("saving", () => {
  it("hands up every frame, its hold and transition, and Loop, and clears the unsaved mark", async () => {
    const onSave = vi.fn();
    show(framed([RED, GREEN], [100, 200]), { onSave });
    await settle();
    fireEvent.click(connector(1));
    fireEvent.click(screen.getByRole("radio", { name: "Fade" }));
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
    });
    const saved = onSave.mock.calls[0]![0] as Pattern;
    expect(saved.frames).toHaveLength(2);
    expect(saved.frames[0]!.transition).toEqual({ kind: "fade", ms: 400, easing: "linear" });
    expect(saved.frames.map((f) => f.hold)).toEqual([100, 200]);
    expect(within(document.body).getByText("No unsaved changes")).toBeTruthy();
  });

  it("an untouched pattern is on the lights as its first frame alone, and Play sends it exactly as it was loaded", async () => {
    const loaded = framed([RED, GREEN], [100, 200]);
    show(loaded);
    await settle();
    expect(sent().frames).toEqual([loaded.frames[0]]);
    await playAll();
    expect(sent()).toEqual(loaded);
    expect(JSON.stringify(sent())).toBe(JSON.stringify(loaded));
  });
});
