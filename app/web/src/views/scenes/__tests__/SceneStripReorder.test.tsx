import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SceneStrip } from "@/views/scenes/SceneStrip";
import type { Scene } from "@/proto/Scene";

// The scene keys can be put in another order (#371): a drag with the mouse, the arrow keys, and
// the scene's own menu for a phone.
//
// The keys never move while a key is dragged: a marker stands in the gap the key would land in,
// and the strip shifts nothing.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom has no PointerEvent and no pointer capture; a MouseEvent carries clientX just as well.
if (!("PointerEvent" in window)) {
  class PE extends MouseEvent {
    pointerId: number;
    pointerType: string;
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 1;
      this.pointerType = init.pointerType ?? "mouse";
    }
  }
  (window as unknown as { PointerEvent: unknown }).PointerEvent = PE;
}

afterEach(() => {
  cleanup();
});

const rect = (left: number, width: number) =>
  ({
    left,
    right: left + width,
    width,
    top: 0,
    bottom: 40,
    height: 40,
    x: left,
    y: 0,
    toJSON: () => ({}),
  }) as DOMRect;

function scene(id: string, name: string): Scene {
  return { id, name, rev: 1, groups: [] };
}

const SCENES = [scene("a", "Night"), scene("b", "Evening"), scene("c", "Morning")];

/** The keys are 100 px wide and 8 px apart, as the strip lays them out. */
function place(el: HTMLElement, left: number, width = 100) {
  el.getBoundingClientRect = () => rect(left, width);
}

/** The strip and its keys, with their places measured. */
function measure(root: HTMLElement) {
  const keys = [...root.querySelectorAll<HTMLElement>("button[data-scene]")];
  const strip = keys[0]?.parentElement as HTMLElement;
  place(strip, 0, 400);
  // The "+" key is pinned at the strip's right end: the keys are all in view, so the strip does
  // not scroll itself.
  const pin = strip.querySelector<HTMLElement>("[data-pin]");
  if (pin) place(pin, 352, 48);
  keys.forEach((key, i) => place(key, 4 + i * 108));
  // The strip revealed the active key before it had a layout (every rect was 0 then), so it
  // scrolled itself. The places are known now: the scroll goes back to where it was.
  strip.scrollLeft = 0;
  return { strip, keys };
}

function setup(onSet = vi.fn(), onReorder = vi.fn()) {
  const view = render(
    <SceneStrip
      scenes={SCENES}
      active="a"
      onSet={onSet}
      onNew={vi.fn()}
      onReorder={onReorder}
    />,
  );
  const { strip, keys } = measure(view.container);
  return { view, strip, keys, onSet, onReorder };
}

/** A press on a key, and a drag that carries it past the threshold. */
function dragFrom(key: HTMLElement, to: number) {
  fireEvent.pointerDown(key, { pointerType: "mouse", clientX: 10, clientY: 20, button: 0 });
  // The button is down while the pointer travels.
  fireEvent.pointerMove(window, { pointerType: "mouse", clientX: to, clientY: 20, buttons: 1 });
}

describe("the scene strip's order", () => {
  it("a drag lifts the key and writes the new order once", () => {
    const { keys, onReorder, onSet } = setup();
    // The first key is dragged past the second one.
    dragFrom(keys[0]!, 200);
    fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 200, clientY: 20 });
    expect(onReorder).toHaveBeenCalledTimes(1);
    expect(onReorder).toHaveBeenCalledWith(["b", "a", "c"]);
    // The click that ends the drag is not a choice of a scene.
    expect(onSet).not.toHaveBeenCalled();
  });

  it("a press that does not travel is a click: the scene is set", () => {
    const { keys, onReorder, onSet } = setup();
    fireEvent.pointerDown(keys[1]!, { pointerType: "mouse", clientX: 120, clientY: 20, button: 0 });
    fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 121, clientY: 20 });
    expect(onReorder).not.toHaveBeenCalled();
    expect(onSet).not.toHaveBeenCalled();
    // The key's own click sets the scene.
    fireEvent.click(keys[1]!);
    expect(onSet).toHaveBeenCalledWith("b");
  });

  it("a finger does not drag: the phone's path is the menu", () => {
    const { keys, onReorder } = setup();
    fireEvent.pointerDown(keys[0]!, { pointerType: "touch", clientX: 10, clientY: 20, button: 0 });
    fireEvent.pointerMove(window, { pointerType: "touch", clientX: 200, clientY: 20, buttons: 1 });
    fireEvent.pointerUp(window, { pointerType: "touch", clientX: 200, clientY: 20 });
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("the drag ends without a write when the window loses the focus", () => {
    const { keys, onReorder } = setup();
    dragFrom(keys[0]!, 200);
    // Another window took the focus: the drag is over.
    fireEvent.blur(window);
    fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 200, clientY: 20 });
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("the drag ends without a write when the pointer is no longer down", () => {
    const { keys, onReorder } = setup();
    dragFrom(keys[0]!, 200);
    // The button was let go of outside the window: the pointer is not down any more.
    fireEvent.pointerMove(window, { pointerType: "mouse", clientX: 220, clientY: 20, buttons: 0 });
    fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 220, clientY: 20 });
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("the drag ends without a write when the pointer is cancelled", () => {
    const { keys, onReorder } = setup();
    dragFrom(keys[0]!, 200);
    fireEvent.pointerCancel(window, { pointerType: "mouse", clientX: 200, clientY: 20 });
    fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 200, clientY: 20 });
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("a key the keyboard takes after a drag is a choice again", () => {
    const { keys, onSet } = setup();
    // The first key is dragged onto the second one: the drop gives no key a click.
    dragFrom(keys[0]!, 200);
    fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 200, clientY: 20 });
    // The keyboard takes the third key. There is no pointer press to clear the flag, so the
    // keydown does it.
    fireEvent.keyDown(keys[2]!, { key: "Enter" });
    fireEvent.click(keys[2]!);
    expect(onSet).toHaveBeenCalledWith("c");
  });

  it("the drag shows a marker in the gap the key would land in", () => {
    const { keys } = setup();
    dragFrom(keys[0]!, 200);
    const marker = screen.getByTestId("strip-marker");
    // The marker stands in the gap between the second key and the third one.
    const second = keys[1]!.getBoundingClientRect();
    const third = keys[2]!.getBoundingClientRect();
    const left = parseFloat(marker.style.left);
    expect(left).toBeGreaterThan(second.right);
    expect(left).toBeLessThan(third.left);
  });

  it("Escape calls the drag off, and nothing is written", () => {
    const { keys, onReorder } = setup();
    dragFrom(keys[0]!, 200);
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.pointerUp(window, { pointerType: "mouse", clientX: 200, clientY: 20 });
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("the arrow keys move the key one place, and the page is told where it is", () => {
    const { keys, onReorder } = setup();
    fireEvent.keyDown(keys[1]!, { key: "ArrowLeft" });
    expect(onReorder).toHaveBeenCalledTimes(1);
    expect(onReorder).toHaveBeenCalledWith(["b", "a", "c"], { coalesce: true });
    // The keyboard is told where the key is.
    expect(screen.getByRole("status").textContent).toBe("Evening, position 1 of 3.");
  });

  it("the arrow keys say when a key is already at an end", () => {
    const { keys, onReorder } = setup();
    fireEvent.keyDown(keys[0]!, { key: "ArrowLeft" });
    expect(onReorder).not.toHaveBeenCalled();
    expect(screen.getByRole("status").textContent).toBe("Night is already first.");
    fireEvent.keyDown(keys[2]!, { key: "ArrowRight" });
    expect(onReorder).not.toHaveBeenCalled();
    expect(screen.getByRole("status").textContent).toBe("Morning is already last.");
  });

  it("every key keeps the shared tab-key class", () => {
    const { keys } = setup();
    for (const key of keys) expect(key.className).toContain("tab-key");
  });
});
