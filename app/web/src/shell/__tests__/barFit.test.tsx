// What leaves the status strip's row, and in what order (#586 B9, B10, B12). jsdom does no
// layout, so each part says its width, and a part the published attributes hide says none.

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type { CSSProperties } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { BarOverflow } from "@/shell/PageHeader";
import { fitBar } from "@/shell/barFit";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let rowWidth = 0;

beforeEach(() => {
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
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get(this: HTMLElement) {
      const flags = document.documentElement.dataset;
      if (flags.barContext === "body" && this.classList.contains("rd-context")) return 0;
      if (flags.barSecondary === "menu" && this.classList.contains("rd-secondary")) return 0;
      const own = this.getAttribute("data-w");
      if (own) return Number(own);
      return [...this.children].reduce((sum, el) => sum + (el as HTMLElement).offsetWidth, 0);
    },
  });
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get: () => rowWidth,
  });
});

afterEach(() => {
  cleanup();
  delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
  delete (HTMLElement.prototype as { clientWidth?: number }).clientWidth;
  delete document.documentElement.dataset.barContext;
  delete document.documentElement.dataset.barSecondary;
});

/** Title 100, context 300, secondary key 90, primary 80: 570 in all. */
function fit(width: number, wraps = false) {
  rowWidth = width;
  const { container } = render(
    <div className="strip">
      <div className="strip-row" style={wraps ? ({ "--bar-wraps": "1" } as CSSProperties) : undefined}>
        <span className="strip-slot">
          <span className="rd-title">
            <h1 data-w="100">Fans</h1>
            <span className="rd-context" data-w="300" />
          </span>
        </span>
        <span className="strip-page-actions">
          <span className="rd-actions">
            <button className="rd-secondary" data-w="90" />
            <button data-bar-primary="" data-w="80" />
          </span>
        </span>
      </div>
    </div>,
  );
  return fitBar(container.querySelector(".strip") as HTMLElement, container.querySelector(".strip-row") as HTMLElement);
}

describe("the strip's fit", () => {
  it("keeps everything that fits in the bar", () => {
    expect(fit(600)).toEqual({ context: "bar", secondary: "bar", rows: "1" });
  });

  it("gives up the context before a key", () => {
    expect(fit(400)).toEqual({ context: "body", secondary: "bar", rows: "1" });
  });

  it("moves the secondary key to the overflow only when it still does not fit", () => {
    expect(fit(200)).toEqual({ context: "body", secondary: "menu", rows: "1" });
  });

  it("takes a second line only for a primary key that cannot fit, and asks both lines again", () => {
    expect(fit(175, true)).toEqual({ context: "body", secondary: "bar", rows: "2" });
  });

  it("stays on one line where the stylesheet does not wrap the row", () => {
    expect(fit(175)).toEqual({ context: "body", secondary: "menu", rows: "1" });
  });
});

describe("the overflow menu", () => {
  it("holds the page's secondary keys once they do not fit the bar", async () => {
    render(
      <BarOverflow>
        <button type="button">Revert</button>
      </BarOverflow>,
    );
    await act(async () => {
      document.documentElement.dataset.barSecondary = "menu";
    });
    fireEvent.click(screen.getByRole("button", { name: "More page actions" }));
    const menu = screen.getByRole("dialog", { name: "More page actions" });
    expect(within(menu).getByRole("button", { name: "Revert" })).toBeTruthy();
  });
});
