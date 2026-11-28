// A name a person typed in the bar (#586 B5, B7). jsdom does no layout, so this file models the
// one thing that matters: the heading, and the box around it, are only as wide as the text they
// draw, up to the room the row has. A measurement taken from either of them can never grow back.

import { act, cleanup, render, screen } from "@testing-library/react";
import type { CSSProperties } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UserTitle } from "@/shell/PageHeader";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** Every character is ten pixels wide, an emoji too. */
const textWidth = (text: string) => Array.from(text).length * 10;
/** What stands beside the name in the row. */
const BESIDE = 200;

let room = 0;
const observers = new Set<() => void>();

const drawnWidth = (el: Element): number => {
  if (el.tagName === "H1") return Math.min(textWidth(el.textContent ?? ""), room);
  if (el.classList.contains("rd-title")) return drawnWidth(el.querySelector("h1") as Element);
  return Number(el.getAttribute("data-w") ?? 0);
};

beforeEach(() => {
  observers.clear();
  globalThis.ResizeObserver = class {
    constructor(private callback: () => void) {}
    observe() {
      observers.add(this.callback);
    }
    unobserve() {}
    disconnect() {
      observers.delete(this.callback);
    }
  } as unknown as typeof ResizeObserver;
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    font: "",
    measureText: (text: string) => ({ width: textWidth(text) }),
  } as unknown as CanvasRenderingContext2D);
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get(this: HTMLElement) {
      return drawnWidth(this);
    },
  });
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("strip-row") ? room + BESIDE : drawnWidth(this);
    },
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return { width: drawnWidth(this) } as DOMRect;
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
  delete (HTMLElement.prototype as { clientWidth?: number }).clientWidth;
});

function mount(title: string, width: number) {
  room = width;
  return render(
    <div className="strip-row">
      <span className="strip-slot">
        <span className="rd-title">
          <UserTitle title={title} />
        </span>
      </span>
      <span data-w={BESIDE} />
    </div>,
  );
}

function resize(width: number) {
  room = width;
  act(() => {
    for (const callback of [...observers]) callback();
  });
}

const drawn = () => screen.getByRole("heading").textContent ?? "";
const LONG = "A very long stack name that somebody typed in the builder by hand";

describe("a user-named title", () => {
  it("is whole again when the window widens after it was cut", () => {
    mount(LONG, 800);
    expect(drawn()).toBe(LONG);
    resize(200);
    expect(drawn()).toContain("…");
    expect(textWidth(drawn())).toBeLessThanOrEqual(200);
    resize(800);
    expect(drawn()).toBe(LONG);
  });

  it("uses the room there is, and keeps its full name for a reader", () => {
    mount(LONG, 300);
    expect(textWidth(drawn())).toBe(300);
    expect(screen.getByRole("heading", { name: LONG }).getAttribute("title")).toBe(LONG);
  });

  it("never drops below twelve characters", () => {
    mount(LONG, 40);
    expect(Array.from(drawn().replace("…", ""))).toHaveLength(12);
  });

  it("keeps the room of now playing once it has moved to the keys' line", () => {
    room = 800;
    render(
      <div className="strip" data-bar-rows="2" data-bar-now="second">
        <div className="strip-row" style={{ "--bar-wraps": "1" } as CSSProperties}>
          <span className="strip-slot">
            <span className="rd-title">
              <UserTitle title={LONG} />
            </span>
          </span>
          <span data-w={BESIDE} />
          <span className="strip-now" data-w={700} />
        </div>
      </div>,
    );
    expect(drawn()).toBe(LONG);
  });

  it("never splits an emoji, at any width", () => {
    const title = `a${"😀".repeat(30)}`;
    const lone = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;
    for (let width = 100; width <= 320; width += 10) {
      mount(title, width);
      expect(drawn(), `at ${width}px`).not.toMatch(lone);
      cleanup();
    }
  });
});
