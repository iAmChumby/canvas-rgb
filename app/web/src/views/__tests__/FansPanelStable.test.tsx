import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { getSnap, patchSnap, refreshFans } from "@/lib/store";
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
    expect(screen.queryByText(/canvas-rgb (overrode|handed)/)).toBeNull();
    quiet.unmount();

    await page("?state=handback:cpu_fan");
    const withReason = { sig: signature(), at: pointsAt(), rows: document.querySelectorAll(".curve-row").length };

    expect(without.rows).toBe(4);
    expect(withReason.rows).toBe(4);
    expect(withReason.sig).toBe(without.sig);
    expect(withReason.at).toBe(without.at);
  });

  it("says the reason under the chart, naming the fan, and never above the floor note", async () => {
    await page("?state=handback:cpu_fan");
    const reason = screen.getByText(/canvas-rgb handed CPU fan back to BIOS: /);
    const region = document.querySelector("[data-slot='fan-status-reason']");
    expect(region?.contains(reason)).toBe(true);
    // The floor note comes first in the page, the reason after it, and the points after both.
    const floor = [...document.querySelectorAll(".rd-fans-right .rule")].find((e) => e.textContent?.startsWith("Floor"));
    expect(floor).toBeTruthy();
    expect(floor!.compareDocumentPosition(region!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(region!.compareDocumentPosition(document.querySelector(".curve-rows")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(document.querySelectorAll("[data-slot='fan-status']")).toHaveLength(1);
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

    fireEvent.keyDown(screen.getByLabelText("Point 2 temperature"), { key: "ArrowUp" });
    await waitFor(() => expect(sawApplying).toBe(true), { timeout: 3000 });
    await waitFor(() => expect(stack().textContent).not.toContain("Applying"), { timeout: 3000 });
    watch.disconnect();

    expect([...seen]).toEqual([before]);
    expect(document.querySelectorAll(".curve-row")).toHaveLength(4);
    expect(document.querySelector(".curve-rows")?.textContent).toContain("of 8 points");
  });

  it("shows an error as an alert and keeps Applying beside it while the request is out", async () => {
    await page("?state=save-error");
    const both: boolean[] = [];
    const watch = new MutationObserver(() => {
      const alert = document.querySelector(".rd-fans-right [role='alert']");
      both.push(!!alert?.textContent && !!stack().textContent?.includes("Applying"));
    });
    watch.observe(stack(), { childList: true, subtree: true, characterData: true });
    fireEvent.keyDown(screen.getByLabelText("Point 2 temperature"), { key: "ArrowUp" });
    const alert = await waitFor(
      () => {
        const el = document.querySelector<HTMLElement>(".rd-fans-right [role='alert']");
        if (!el?.textContent) throw new Error("no error yet");
        return el;
      },
      { timeout: 3000 },
    );
    watch.disconnect();
    expect(alert.getAttribute("role")).toBe("alert");
    expect(both).toContain(true);
  });

  it("announces one stable sentence per kind of reason, not the temperature inside it", async () => {
    await page("");
    const live = () => document.querySelector("[data-slot='fan-status-announce']") as HTMLElement;
    expect(live().getAttribute("aria-live")).toBe("polite");
    expect(live().textContent).toBe("");

    const withReason = (reason: string, onBios: boolean) => {
      const fans = getSnap().fans!;
      act(() => {
        patchSnap({
          fans: { ...fans, headers: fans.headers.map((h) => (h.id === "cpu_fan" ? { ...h, on_bios: onBios, override_reason: reason } : h)) },
        });
      });
    };
    withReason("hot floor 50% while CPU is 78 °C", false);
    const first = live().textContent;
    expect(first).toBe("canvas-rgb is overriding CPU fan");
    // Only the temperature changes: the announced words stay as they were.
    withReason("hot floor 50% while CPU is 79 °C", false);
    expect(live().textContent).toBe(first);
    // The words the eye reads do carry the live value, and sit outside the live region.
    expect(screen.getByText(/while CPU is 79/)).toBeTruthy();
    expect(live().contains(screen.getByText(/while CPU is 79/))).toBe(false);
    // A different kind of reason is announced.
    withReason("write failed: busy", true);
    expect(live().textContent).toBe("CPU fan: canvas-rgb handed this header back to BIOS");
  });

  it("says who is in charge in the same box when there is no reason, and announces nothing", async () => {
    await page("");
    const region = document.querySelector("[data-slot='fan-status-reason']")!;
    expect(region.textContent).toBe("Curve in charge: canvas-rgb has not changed this fan.");
    expect(region.querySelector(".rule-warn")).toBeNull();
    expect(document.querySelector("[data-slot='fan-status-announce']")?.textContent).toBe("");
  });

  it("offers More only for a reason that is cut, and opens it over the page without adding a block", async () => {
    // jsdom has no layout: the text reports that it is taller than its two lines.
    const sh = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
    const ch = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get(this: HTMLElement) { return this.classList.contains("fan-status-text") ? 120 : (sh?.get?.call(this) as number) ?? 0; } });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get(this: HTMLElement) { return this.classList.contains("fan-status-text") ? 40 : (ch?.get?.call(this) as number) ?? 0; } });
    try {
      await page("?state=handback:cpu_fan");
      const before = signature();
      const more = await screen.findByRole("button", { name: "More" });
      expect(more.getAttribute("aria-expanded")).toBe("false");
      fireEvent.click(more);
      const less = screen.getByRole("button", { name: "Less" });
      expect(less.getAttribute("aria-expanded")).toBe("true");
      expect(document.querySelector(".fan-status-box.is-open")).not.toBeNull();
      expect(signature()).toBe(before);
      fireEvent.keyDown(less, { key: "Escape" });
      expect(screen.getByRole("button", { name: "More" })).toBeTruthy();
    } finally {
      if (sh) Object.defineProperty(HTMLElement.prototype, "scrollHeight", sh); else delete (HTMLElement.prototype as unknown as Record<string, unknown>).scrollHeight;
      if (ch) Object.defineProperty(HTMLElement.prototype, "clientHeight", ch); else delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
    }
  });

  // Last: it switches the demo's CPU fan to Fixed, which the other tests do not expect.
  it("keeps the same live node and the same words across a switch to Fixed, and the reason stands in Fixed too", async () => {
    await page("?state=handback:cpu_fan");
    const live = document.querySelector("[data-slot='fan-status-announce']")!;
    const words = live.textContent;
    expect(words).toBe("CPU fan: canvas-rgb handed this header back to BIOS");

    fireEvent.click(screen.getByRole("radio", { name: "Fixed %" }));
    await screen.findByLabelText("CPU fan duty value");
    await waitFor(() => expect(screen.getByRole("radio", { name: "Fixed %" }).getAttribute("aria-checked")).toBe("true"));
    // The same element, the same words: nothing was mounted again for a screen reader to say twice.
    expect(document.querySelector("[data-slot='fan-status-announce']")).toBe(live);
    expect(live.textContent).toBe(words);
    // Fixed has the region too: the floor note, then the Duty field, then the reason.
    const region = document.querySelector("[data-slot='fan-status-reason']")!;
    expect(region.textContent).toContain("canvas-rgb handed CPU fan back to BIOS: ");
    expect(document.querySelectorAll("[data-slot='fan-status']")).toHaveLength(1);
    const duty = screen.getByLabelText("CPU fan duty value");
    expect(duty.compareDocumentPosition(region) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
