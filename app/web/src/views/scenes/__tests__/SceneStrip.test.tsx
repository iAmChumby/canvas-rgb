import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { SceneStrip, revealActiveKey } from "@/views/scenes/SceneStrip";
import type { Scene } from "@/proto/Scene";

// The scene strip scrolls sideways when its keys outrun the window, and the "+" key stays pinned at
// its right end. The key that says what is playing is never left hidden behind either edge.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
});

const rect = (left: number, width: number) =>
  ({ left, right: left + width, width, top: 0, bottom: 40, height: 40, x: left, y: 0, toJSON: () => ({}) }) as DOMRect;

/** A strip 300 px wide at x=0, its "+" pin 48 px wide, one active key at `keyLeft`/`keyWidth`. */
function fakeStrip(keyLeft: number, keyWidth: number, scrollLeft = 0) {
  const strip = document.createElement("div");
  strip.getBoundingClientRect = () => rect(0, 300);
  let scroll = scrollLeft;
  Object.defineProperty(strip, "scrollLeft", { get: () => scroll, set: (v: number) => void (scroll = v) });
  const key = document.createElement("button");
  key.setAttribute("aria-pressed", "true");
  key.getBoundingClientRect = () => rect(keyLeft - (scroll - scrollLeft), keyWidth);
  const pin = document.createElement("span");
  pin.setAttribute("data-pin", "");
  pin.getBoundingClientRect = () => rect(252, 48);
  strip.append(key, pin);
  return { strip, key, scroll: () => scroll };
}

describe("revealActiveKey", () => {
  it("leaves the strip alone when the active key is in view", () => {
    const { strip, scroll } = fakeStrip(20, 90);
    revealActiveKey(strip);
    expect(scroll()).toBe(0);
  });

  it("scrolls right until a key behind the pinned '+' clears it, with a margin", () => {
    // key spans 230..330: past the pin's reach (300 - 48 - 4 = 248) by 82
    const { strip, key, scroll } = fakeStrip(230, 100);
    revealActiveKey(strip);
    expect(scroll()).toBe(82);
    expect(key.getBoundingClientRect().right).toBeLessThanOrEqual(248);
  });

  it("scrolls left when the active key sits behind the strip's left edge", () => {
    const { strip, key, scroll } = fakeStrip(-40, 100, 120);
    revealActiveKey(strip);
    expect(key.getBoundingClientRect().left).toBeGreaterThanOrEqual(4);
    expect(scroll()).toBeLessThan(120);
  });

  it("does nothing when no key is active", () => {
    const strip = document.createElement("div");
    expect(() => revealActiveKey(strip)).not.toThrow();
  });
});

describe("the scene strip", () => {
  const scene = (id: string): Scene => ({ id, name: id, groups: [] }) as unknown as Scene;

  it("marks the pin and the active key the reveal looks for, and mounts without a layout engine", () => {
    const scenes = ["a", "b", "c", "d"].map(scene);
    const view = render(<SceneStrip scenes={scenes} active="d" onSet={() => {}} onNew={() => {}} />);
    expect(view.container.querySelector("[data-pin]")).not.toBeNull();
    expect(view.container.querySelector('[aria-pressed="true"]')?.textContent).toBe("d");
    // The reveal runs on mount and on a switch: with no layout (every rect is 0) it must not throw.
    expect(() => view.rerender(<SceneStrip scenes={scenes} active="a" onSet={() => {}} onNew={() => {}} />)).not.toThrow();
    expect(view.container.querySelector('[aria-pressed="true"]')?.textContent).toBe("a");
  });
});
