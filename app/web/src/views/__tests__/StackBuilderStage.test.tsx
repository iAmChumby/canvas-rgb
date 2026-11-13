// The builder's stage (#257 phase 1) against the demo backend: every layer's frame is drawn, a
// press picks the smallest one under it, Tab and Escape walk the pick, Delete removes the picked
// layer, Ctrl+D copies it, the Make-rectangle key converts an area by name, and undo and redo
// take a whole drag back in one press.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { dismissNotifications, NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StackAreaPicker } from "@/views/effects/StackAreaPicker";
import { canvasWith } from "@/lib/__tests__/groupCanvasFixture";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

vi.mock("@/lib/stackPreview", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/stackPreview")>();
  return {
    ...actual,
    onStackPreviewFeed: () => () => {},
  };
});

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/effects/",
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=stack");
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
  Range.prototype.getClientRects = (() =>
    ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList) as typeof Range.prototype.getClientRects;
  Range.prototype.getBoundingClientRect = (() =>
    ({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) }) as DOMRect) as typeof Range.prototype.getBoundingClientRect;
});

afterEach(async () => {
  cleanup();
  vi.restoreAllMocks();
  act(() => dismissNotifications());
  // A change undone back to the saved stack drops the builder's guard entry with
  // `history.back()`, which lands a turn later: let it land before the next test sets its URL.
  await new Promise((resolve) => setTimeout(resolve, 0));
});

/** The stage is mocked at 640 x 400 css pixels: one canvas unit is two pixels. */
const K = 2;
const px = (u: number) => u * K;

async function openBuilder() {
  await refreshAll();
  // The URL is set after the read and just before the mount: a guard entry popped by the
  // previous test lands during the read, and it would otherwise win over this one.
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
  render(
    <>
      <NotifyHost />
      <StripSlotsProvider>
        <div className="strip-slots">
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </div>
        <EffectsView />
      </StripSlotsProvider>
    </>,
  );
  await screen.findByTestId("stack-builder");
  await screen.findByRole("heading", { name: "Sunset glow" });
  await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(3));
  const stage = document.querySelector<HTMLElement>(".rd-canvas")!;
  vi.spyOn(stage, "getBoundingClientRect").mockReturnValue({
    x: 0, y: 0, left: 0, top: 0, right: 640, bottom: 400, width: 640, height: 400, toJSON: () => ({}),
  } as DOMRect);
  return stage;
}

const row = (name: string) => screen.getByText(name).closest("li") as HTMLElement;
const pickedName = () => {
  const panel = document.querySelector(".rd-layer-edit");
  if (!panel || panel.classList.contains("rd-layer-none")) return null;
  return panel.querySelector("h2")?.textContent ?? null;
};
const readout = () => document.querySelector(".layer-area-text")?.textContent ?? "";
const frameOf = (id: string) => document.querySelector<HTMLElement>(`.rd-layer-frame[data-layer-id="${id}"]`);

const press = (stage: HTMLElement, x: number, y: number) => {
  act(() => {
    fireEvent.pointerDown(stage, { pointerId: 1, pointerType: "mouse", button: 0, clientX: x, clientY: y });
    fireEvent.pointerUp(stage, { pointerId: 1, pointerType: "mouse", buttons: 0, clientX: x, clientY: y });
  });
};

const mouse = { pointerId: 1, pointerType: "mouse", buttons: 1 };

function dragOn(el: HTMLElement, from: [number, number], to: [number, number]) {
  act(() => {
    fireEvent.pointerDown(el, { ...mouse, clientX: from[0], clientY: from[1] });
    fireEvent.pointerMove(el, { ...mouse, clientX: (from[0] + to[0]) / 2, clientY: (from[1] + to[1]) / 2 });
    fireEvent.pointerMove(el, { ...mouse, clientX: to[0], clientY: to[1] });
    fireEvent.pointerUp(el, { ...mouse, buttons: 0, clientX: to[0], clientY: to[1] });
  });
}

const zoneLayer = {
  id: "l1",
  name: "Layer l1",
  effect: "solid",
  props: {},
  area: { kind: "zone", device: "kb:1", zone: 2 },
  blend: "normal",
  opacity: 100,
  visible: true,
} as const;

describe("the stage", () => {
  it("draws the group it is tuning and names it", async () => {
    await refreshAll();
    window.history.replaceState(
      {},
      "",
      "/effects/?stack=Sunset%20glow&state=complete,stack&group=g-case",
    );
    render(
      <StripSlotsProvider>
        <div className="strip-slots">
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </div>
        <EffectsView />
      </StripSlotsProvider>,
    );
    await screen.findByTestId("stack-builder");
    // The panel says which group the picture is for.
    expect(await screen.findByText(/Preview · Case fans/i)).toBeTruthy();
    // And the group's lights are drawn on it: a slot outline for each of its three fans.
    await waitFor(() => expect(document.querySelectorAll(".rd-part-outline")).toHaveLength(3));
    expect(screen.getByRole("switch", { name: "Show each light's LEDs" }).getAttribute("aria-checked")).toBe("true");
  });

  it("offers the canvas, a device in the group and a rectangle, and no zone", async () => {
    await openBuilder();
    fireEvent.click(screen.getByRole("button", { name: "Change area" }));
    const picker = await screen.findByRole("dialog", { name: "Area for Sparkles" });
    const modes = within(picker).getAllByRole("radio").map((el) => el.textContent);
    expect(modes).toEqual(["Whole canvas", "Device", "Rectangle"]);
    expect(within(picker).queryByRole("radio", { name: "Zone" })).toBeNull();
  });

  it("keeps a saved zone area as a zone, and says which one", async () => {
    const canvas = canvasWith([
      { id: "kb:1#z2", label: "Keyboard · Zone 3", device: "kb:1", zone: 2, x: 0, y: 0, width: 100, height: 50 },
    ]);
    const onUse = vi.fn();
    render(
      <StackAreaPicker
        layer={zoneLayer}
        canvas={canvas}
        devices={null}
        onUse={onUse}
        onClose={() => undefined}
      />,
    );
    const picker = await screen.findByRole("dialog", { name: "Area for Layer l1" });
    const zone = within(picker).getByRole("radio", { name: "Zone" });
    expect(zone.getAttribute("aria-checked")).toBe("true");
    expect(within(picker).getAllByText("Keyboard · Zone 3").length).toBeGreaterThan(0);
    fireEvent.click(within(picker).getByRole("button", { name: "Use this area" }));
    expect(onUse).toHaveBeenCalledWith({ kind: "zone", device: "kb:1", zone: 2 });
  });

  it("says when the group the builder was opened for is not in the scene", async () => {
    await refreshAll();
    window.history.replaceState(
      {},
      "",
      "/effects/?stack=Sunset%20glow&state=complete,stack&group=gone",
    );
    render(
      <StripSlotsProvider>
        <div className="strip-slots">
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </div>
        <EffectsView />
      </StripSlotsProvider>,
    );
    await screen.findByTestId("stack-builder");
    expect(await screen.findByText("That group is not in the active scene.")).toBeTruthy();
  });

  it("draws a frame for every visible layer, and only the picked one is the solid frame", async () => {
    await openBuilder();
    expect(frameOf("l1")).toBeTruthy();
    expect(frameOf("l2")).toBeTruthy();
    expect(frameOf("l3")).toBeTruthy();
    // The builder opens on the top layer (Sparkles); only its frame is the solid one.
    expect(frameOf("l3")?.classList.contains("rd-area-frame")).toBe(true);
    expect(frameOf("l1")?.classList.contains("rd-area-frame")).toBe(false);
    expect(document.querySelectorAll(".rd-area-frame")).toHaveLength(1);
  });

  it("picks the smallest frame under the press", async () => {
    const stage = await openBuilder();
    // The middle of Heat's 110 x 46 rectangle: inside it and inside both canvas layers.
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    // A point outside Heat's box: the whole-canvas layers tie, and the top one wins.
    press(stage, px(200), px(40));
    await waitFor(() => expect(pickedName()).toBe("Sparkles"));
  });

  it("walks the layers with Tab and Shift+Tab, top first", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    fireEvent.keyDown(stage, { key: "Tab" });
    await waitFor(() => expect(pickedName()).toBe("Sunset"));
    fireEvent.keyDown(stage, { key: "Tab", shiftKey: true });
    await waitFor(() => expect(pickedName()).toBe("Heat"));
  });

  it("clears the pick with Escape", async () => {
    const stage = await openBuilder();
    fireEvent.keyDown(stage, { key: "Escape" });
    await waitFor(() => expect(pickedName()).toBeNull());
    // With nothing picked, Tab is the browser's again: the stage does not cycle.
    fireEvent.keyDown(stage, { key: "Tab" });
    expect(pickedName()).toBeNull();
  });

  it("removes the picked layer with Delete, and brings it back with undo", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    fireEvent.keyDown(stage, { key: "Delete" });
    await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(2));
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(3));
    expect(screen.getByText("Heat")).toBeTruthy();
  });

  it("copies the picked layer with Ctrl+D", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    fireEvent.keyDown(stage, { key: "d", ctrlKey: true });
    await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(4));
    expect(pickedName()).toBe("Heat copy");
  });

  it("takes a whole drag back with one undo, and forward with redo", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(readout()).toContain("x 16, y 150"));
    const frame = document.querySelector<HTMLElement>(".rd-area-frame")!;
    dragOn(frame, [px(71), px(173)], [px(71) + 40, px(173) - 20]);
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    // One undo puts the whole drag back, not one frame of it.
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    await waitFor(() => expect(readout()).toContain("x 16, y 150"));
    expect(screen.queryByText("Unsaved")).toBeNull();
    // Redo takes it forward again.
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    // Ctrl+Y is the other redo; with nothing to redo it changes nothing.
    fireEvent.keyDown(document.body, { key: "y", ctrlKey: true });
    expect(readout()).toContain("x 36, y 140");
  });

  it("keeps Ctrl+Z for the text field it was typed in", async () => {
    await openBuilder();
    // The handler is on the window; any field on the page keeps its own undo.
    const input = document.createElement("input");
    document.body.append(input);
    try {
      expect(fireEvent.keyDown(input, { key: "z", ctrlKey: true })).toBe(true);
    } finally {
      input.remove();
    }
    expect(document.querySelectorAll(".layer-row")).toHaveLength(3);
  });

  it("offers Make rectangle on a whole-canvas area and converts it by name", async () => {
    const stage = await openBuilder();
    press(stage, px(200), px(40));
    await waitFor(() => expect(pickedName()).toBe("Sparkles"));
    const chip = screen.getByRole("button", { name: "Make Sparkles a rectangle" });
    fireEvent.click(chip);
    await waitFor(() => expect(readout()).toContain("Rectangle 320 × 200"));
    // The area is a rectangle now, so the key is gone and the frame carries handles.
    expect(screen.queryByRole("button", { name: "Make Sparkles a rectangle" })).toBeNull();
    expect(document.querySelector(".rd-area-frame")?.getAttribute("data-editable")).toBe("true");
    // Undo puts the whole-canvas area back.
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    await waitFor(() => expect(readout()).toContain("Whole canvas"));
    expect(screen.getByRole("button", { name: "Make Sparkles a rectangle" })).toBeTruthy();
  });

  it("starts with the LEDs on, and shows and hides each light's outlines with the switch", async () => {
    await openBuilder();
    const toggle = screen.getByRole("switch", { name: "Show each light's LEDs" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await waitFor(() => expect(document.querySelectorAll(".rd-part-outline").length).toBeGreaterThan(0));
    fireEvent.click(toggle);
    await waitFor(() => expect(document.querySelectorAll(".rd-part-outline")).toHaveLength(0));
    fireEvent.click(toggle);
    await waitFor(() => expect(document.querySelectorAll(".rd-part-outline").length).toBeGreaterThan(0));
  });

  it("removes a layer from its row's Delete key too", async () => {
    await openBuilder();
    fireEvent.keyDown(row("Heat"), { key: "Delete" });
    await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(2));
  });
});

// ---- fix pass 1 (#257): drags, guards, nudges, refusals, frames ----------------------------

const rows = () => document.querySelectorAll(".layer-row").length;
// In act, so the keys' updates land before the next line reads the page.
const undoKey = () => act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
const HEAT_AT = "x 16, y 150";

/** Heat picked, its frame ready to drag. */
async function pickHeat() {
  const stage = await openBuilder();
  press(stage, px(71), px(173));
  await waitFor(() => expect(readout()).toContain(HEAT_AT));
  return { stage, frame: document.querySelector<HTMLElement>(".rd-area-frame")! };
}

describe("a drag is one undo step", () => {
  it("however long it pauses in the middle", async () => {
    const { frame } = await pickHeat();
    // Only the clock is faked, so the waits below keep their own timers.
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(new Date(1_000_000));
      act(() => {
        fireEvent.pointerDown(frame, { ...mouse, clientX: px(71), clientY: px(173) });
        fireEvent.pointerMove(frame, { ...mouse, clientX: px(71) + 10, clientY: px(173) - 4 });
      });
      // 1.5 s is longer than the history's merge gap: a drag that is not held would split here.
      vi.setSystemTime(new Date(1_001_500));
      act(() => {
        fireEvent.pointerMove(frame, { ...mouse, clientX: px(71) + 40, clientY: px(173) - 20 });
        fireEvent.pointerUp(frame, { ...mouse, buttons: 0, clientX: px(71) + 40, clientY: px(173) - 20 });
      });
    } finally {
      vi.useRealTimers();
    }
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    undoKey();
    await waitFor(() => expect(readout()).toContain(HEAT_AT));
  });

  it("and a Ctrl+Z in the middle of it waits for the release", async () => {
    const { frame } = await pickHeat();
    act(() => {
      fireEvent.pointerDown(frame, { ...mouse, clientX: px(71), clientY: px(173) });
      fireEvent.pointerMove(frame, { ...mouse, clientX: px(71) + 40, clientY: px(173) - 20 });
    });
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    undoKey();
    expect(readout()).toContain("x 36, y 140");
    act(() => {
      fireEvent.pointerUp(frame, { ...mouse, buttons: 0, clientX: px(71) + 40, clientY: px(173) - 20 });
    });
    undoKey();
    await waitFor(() => expect(readout()).toContain(HEAT_AT));
  });

  it("shows the group's outlines while it lasts, even with the LEDs off", async () => {
    const { frame } = await pickHeat();
    fireEvent.click(screen.getByRole("switch", { name: "Show each light's LEDs" }));
    await waitFor(() => expect(document.querySelectorAll(".rd-part-outline")).toHaveLength(0));
    act(() => {
      fireEvent.pointerDown(frame, { ...mouse, clientX: px(71), clientY: px(173) });
    });
    await waitFor(() => expect(document.querySelectorAll(".rd-part-outline").length).toBeGreaterThan(0));
    act(() => {
      fireEvent.pointerUp(frame, { ...mouse, buttons: 0, clientX: px(71), clientY: px(173) });
    });
    await waitFor(() => expect(document.querySelectorAll(".rd-part-outline")).toHaveLength(0));
  });
});

describe("the window shortcuts", () => {
  it("wait while a library tile is lifted", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    fireEvent.keyDown(stage, { key: "Delete" });
    await waitFor(() => expect(rows()).toBe(2));
    // A drag owns the keyboard: Ctrl+Z would bring Heat back and Ctrl+D would copy the picked
    // layer; neither happens while the tile is lifted.
    const solid = screen.getByRole("button", { name: "Add Solid" });
    fireEvent.keyDown(solid, { key: " " });
    undoKey();
    act(() => void fireEvent.keyDown(document.body, { key: "d", ctrlKey: true }));
    expect(rows()).toBe(2);
    // Escape is the drag's own cancel, and the keys are the page's again.
    fireEvent.keyDown(window, { key: "Escape" });
    undoKey();
    await waitFor(() => expect(rows()).toBe(3));
  });

  it("leave a select and a combobox trigger their own keys", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    const trigger = screen.getAllByRole("combobox")[0]!;
    fireEvent.keyDown(trigger, { key: "d", ctrlKey: true });
    expect(rows()).toBe(3);
    const select = document.createElement("select");
    document.body.append(select);
    try {
      fireEvent.keyDown(select, { key: "d", ctrlKey: true });
    } finally {
      select.remove();
    }
    expect(rows()).toBe(3);
  });

  it("take two steps back when two Ctrl+Z come in the same tick", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    fireEvent.keyDown(stage, { key: "d", ctrlKey: true });
    await waitFor(() => expect(rows()).toBe(4));
    fireEvent.keyDown(stage, { key: "d", ctrlKey: true });
    await waitFor(() => expect(rows()).toBe(5));
    // Both keys reach the window before React has rendered between them: the second must
    // build on the first, not on the draft the first one started from.
    act(() => {
      fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
      fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    });
    await waitFor(() => expect(rows()).toBe(3));
  });

  it("see a change and the Ctrl+Z that follows it", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    act(() => {
      fireEvent.keyDown(stage, { key: "Delete" });
      fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    });
    await waitFor(() => expect(rows()).toBe(3));
    expect(screen.getByText("Heat")).toBeTruthy();
  });
});

describe("arrow nudges", () => {
  const arrow = (frame: HTMLElement, repeat: boolean) =>
    fireEvent.keyDown(frame, { key: "ArrowRight", repeat });

  it("fold the repeats of one held key into one step", async () => {
    const { frame } = await pickHeat();
    arrow(frame, false);
    arrow(frame, true);
    arrow(frame, true);
    arrow(frame, true);
    fireEvent.keyUp(frame, { key: "ArrowRight" });
    await waitFor(() => expect(readout()).toContain("x 20, y 150"));
    undoKey();
    await waitFor(() => expect(readout()).toContain(HEAT_AT));
  });

  it("keep separate presses as separate steps", async () => {
    const { frame } = await pickHeat();
    arrow(frame, false);
    fireEvent.keyUp(frame, { key: "ArrowRight" });
    arrow(frame, false);
    fireEvent.keyUp(frame, { key: "ArrowRight" });
    await waitFor(() => expect(readout()).toContain("x 18, y 150"));
    undoKey();
    await waitFor(() => expect(readout()).toContain("x 17, y 150"));
    undoKey();
    await waitFor(() => expect(readout()).toContain(HEAT_AT));
  });
});

describe("a press where the stage's height is clamped", () => {
  it("maps each corner of a known rectangle to its layer", async () => {
    const stage = await openBuilder();
    // 640 wide but only 300 tall: two px per unit across and 1.5 down, as a short window gives.
    vi.spyOn(stage, "getBoundingClientRect").mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 640, bottom: 300, width: 640, height: 300, toJSON: () => ({}),
    } as DOMRect);
    // Heat is x 16..126, y 150..196. Each press lands one unit inside a corner, after a press
    // outside the box has picked another layer, so a miss shows.
    const inside: [string, number, number][] = [
      ["top left", 17, 151],
      ["top right", 125, 151],
      ["bottom left", 17, 195],
      ["bottom right", 125, 195],
    ];
    for (const [corner, ux, uy] of inside) {
      press(stage, 2 * 200, 1.5 * 40);
      await waitFor(() => expect(pickedName()).toBe("Sparkles"));
      press(stage, 2 * ux, 1.5 * uy);
      await waitFor(() => expect(`${corner}: ${pickedName()}`).toBe(`${corner}: Heat`));
    }
  });
});

describe("a refusal from a key on the stage", () => {
  it("is also a note when the stack holds eight layers", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    for (let n = 4; n <= 8; n++) {
      fireEvent.keyDown(stage, { key: "d", ctrlKey: true });
      await waitFor(() => expect(rows()).toBe(n));
    }
    fireEvent.keyDown(stage, { key: "d", ctrlKey: true });
    await waitFor(() =>
      expect(document.querySelector("[data-sonner-toast]")?.textContent ?? "").toContain("A stack holds 8 layers"),
    );
    expect(rows()).toBe(8);
  });
});

describe("the frames and their handles", () => {
  it("name their kind, so a whole-canvas layer gets its inset frame", async () => {
    await openBuilder();
    expect(frameOf("l1")?.getAttribute("data-kind")).toBe("canvas");
    expect(frameOf("l2")?.getAttribute("data-kind")).toBe("rect");
  });

  it("draw a bar on each of the four sides as well as the four corners", async () => {
    await pickHeat();
    const frame = document.querySelector<HTMLElement>(".rd-area-frame")!;
    expect(frame.querySelectorAll(".handle")).toHaveLength(4);
    const edges = [...frame.querySelectorAll<HTMLElement>(".edge")];
    expect(edges).toHaveLength(4);
    expect(edges.every((edge) => edge.getAttribute("data-mark") === "on")).toBe(true);
  });

  it("carry the shortcut in the undo and redo tooltips", async () => {
    await openBuilder();
    const tip = (name: string) =>
      screen.getByRole("button", { name }).closest(".tip")?.querySelector(".tip-bubble")?.textContent;
    expect(tip("Undo the last change")).toBe("Undo the last change (Ctrl+Z)");
    expect(tip("Redo the change that was undone")).toBe("Redo the change that was undone (Ctrl+Shift+Z)");
  });
});
