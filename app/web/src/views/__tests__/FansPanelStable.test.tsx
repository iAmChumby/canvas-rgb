import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { patchSnap, refreshFans } from "@/lib/store";
import { FansView } from "@/views/FansView";
import { StripSlotsProvider } from "@/shell/PageHeader";

// Issue #109: editing a fan curve made the editor panel grow and shrink. The daemon's reason
// ("canvas-rgb overrode this: hot floor 50%...") appeared above the floor note and pushed the
// panel down, and "Applying" appeared under the points on every commit, so the panel's scrollbar
// came and went and the points were laid out again at a new width.
//
// jsdom has no layout, so these tests hold the thing that makes the layout stable: the editor's
// own children are the same set at every moment of an edit, with the reason or without it. The
// widths and heights are measured in a browser (the issue's screenshots).
//
// The demo backend answers; `?state=handback:cpu_fan` makes it give the CPU fan a reason.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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
      return this.classList.contains("content") ? 1100 : 0;
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

afterEach(() => {
  cleanup();
  patchSnap({ fans: null, fansError: null });
});

/** The fans page, on the CPU fan (the first header, picked on a desktop), in Curve mode. */
async function page(state: string) {
  window.history.replaceState({}, "", `/fans/${state}`);
  await act(async () => {
    await refreshFans();
  });
  const view = render(
    <ToastProvider>
      <StripSlotsProvider>
        <main className="content">
          <FansView />
        </main>
      </StripSlotsProvider>
    </ToastProvider>,
  );
  await screen.findByRole("radiogroup", { name: "CPU fan mode" });
  await waitFor(() => expect(document.querySelectorAll(".curve-row").length).toBeGreaterThan(0));
  return view;
}

/** The editor's stack: every block of the panel, in order. */
function stack(): HTMLElement {
  const el = document.querySelector<HTMLElement>(".rd-fans-right .panel .stack");
  if (!el) throw new Error("no editor stack");
  return el;
}

/** What the panel is made of, block by block: a block that comes or goes changes this. */
function signature(): string {
  return [...stack().children].map((c) => `${c.tagName.toLowerCase()}.${c.className}`).join(" | ");
}

/** Where the points start, counted in blocks from the top of the stack. */
function pointsAt(): number {
  const rows = stack().querySelector(".curve-rows");
  return [...stack().children].findIndex((c) => c.contains(rows));
}

describe("the fan editor panel while the daemon has a reason", () => {
  it("keeps four point rows, the same blocks and the same place for the points with the reason and without it", async () => {
    const quiet = await page("");
    const without = { sig: signature(), at: pointsAt(), rows: document.querySelectorAll(".curve-row").length };
    const emptySlot = document.querySelector("[data-slot='fan-status-reason']")?.textContent;
    expect(screen.queryByText(/canvas-rgb (overrode|handed)/)).toBeNull();
    quiet.unmount();

    await page("?state=handback:cpu_fan");
    // The reason is said, on the page.
    expect(screen.getByText(/canvas-rgb handed this header back to BIOS/)).toBeTruthy();
    const withReason = { sig: signature(), at: pointsAt(), rows: document.querySelectorAll(".curve-row").length };

    expect(without.rows).toBe(4);
    expect(withReason.rows).toBe(4);
    expect(withReason.sig).toBe(without.sig);
    expect(withReason.at).toBe(without.at);
    // With no reason the status region is still in the page, empty: its room is reserved.
    expect(emptySlot).toBe("");
  });

  it("keeps every block of the panel from the first moment of an edit to the last, Applying included", async () => {
    await page("?state=handback:cpu_fan");
    const before = signature();
    const seen = new Set<string>([before]);
    let sawApplying = false;
    const watch = new MutationObserver(() => {
      seen.add(signature());
      if (stack().textContent?.includes("Applying")) sawApplying = true;
    });
    watch.observe(stack(), { childList: true, subtree: true, characterData: true });

    // A change to point 2's temperature: the commit goes out after 250 ms and the page says
    // Applying while it is out.
    fireEvent.keyDown(screen.getByLabelText("Point 2 temperature"), { key: "ArrowUp" });
    await waitFor(() => expect(sawApplying).toBe(true), { timeout: 3000 });
    await waitFor(() => expect(stack().textContent).not.toContain("Applying"), { timeout: 3000 });
    watch.disconnect();

    expect(sawApplying).toBe(true);
    expect([...seen]).toEqual([before]);
    expect(document.querySelectorAll(".curve-row")).toHaveLength(4);
    expect(document.querySelector(".curve-rows")?.textContent).toContain("of 8 points");
  });
});
