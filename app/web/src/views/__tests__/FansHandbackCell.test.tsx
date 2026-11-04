import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { patchSnap, refreshFans } from "@/lib/store";
import { FansView } from "@/views/FansView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// A handed-back header's cell (#114). The name, its chip and the reason used to sit on one
// `white-space: nowrap` line, so a long reason widened the Headers table past its pane and cut
// off the Mode and Last 60 s columns. jsdom does no layout: this test checks the cell's structure
// and the stylesheet; the width itself is measured by .build/handback-width.mjs (headless
// Chromium; gitignored, so in the main checkout).

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const pageWidth = 1100;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("content") ? pageWidth : 0;
    },
  });
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


const REASON = "write failed: readback 0% for 39% after 3000 ms";
const css = readFileSync(resolve(process.cwd(), "src/styles/app.css"), "utf8")
  .replace(/\r\n/g, "\n")
  .replace(/\/\*[\s\S]*?\*\//g, "");

afterEach(() => {
  cleanup();
  window.history.replaceState({}, "", "/fans/");
  patchSnap({ fans: null, fansError: null });
});

describe("a handed-back header's name cell (#114)", () => {
  it("keeps the chip and the reason inside .fan-name-cell, which wraps instead of widening", async () => {
    window.history.replaceState({}, "", `/fans/?state=${encodeURIComponent(`handback:cpu_fan,handback:sys_fan3,handback-reason:${REASON}`)}`);
    await act(async () => {
      await refreshFans();
    });
    const { container } = render(
      <ToastProvider>
        <StripSlotsProvider>
          <header>
            <StripSlot kind="title" />
            <StripSlot kind="actions" />
          </header>
          <main className="content">
            <FansView />
          </main>
        </StripSlotsProvider>
      </ToastProvider>,
    );
    await waitFor(() => expect(container.querySelectorAll("td.name-col .fan-handoff").length).toBe(2));
    for (const cell of container.querySelectorAll("td.name-col")) {
      if (!cell.querySelector(".fan-handoff")) continue;
      const box = cell.querySelector(".fan-name-cell");
      expect(box).not.toBeNull();
      expect(box?.querySelector(".fan-handoff")).not.toBeNull();
      expect(box?.querySelector(".fan-reason")?.textContent).toBe(REASON);
    }
  });

  it("no rule makes the name cell nowrap, and the cell is flex-wrap with a width cap and a gap", () => {
    // Every rule that names td.name-col or th.name-col, at any nesting.
    for (const m of css.matchAll(/([^{}]*name-col[^{}]*)\{([^{}]*)\}/g)) {
      expect(m[2] ?? "", (m[1] ?? "").trim()).not.toMatch(/white-space\s*:\s*nowrap/);
    }
    const cell = /\.rd-fans \.fan-name-cell\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
    expect(cell).toMatch(/flex-wrap\s*:\s*wrap/);
    expect(cell).toMatch(/max-width\s*:/);
    expect(cell).toMatch(/gap\s*:/);
    const reason = /\.rd-fans \.fan-name-cell \.fan-reason\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
    expect(reason).toMatch(/white-space\s*:\s*normal/);
  });
});
