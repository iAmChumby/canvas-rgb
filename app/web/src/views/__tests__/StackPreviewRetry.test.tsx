import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// Issue #97, section 1, on the stack builder: the same uncapped wait, on the same route. The
// daemon says "busy, try again in a moment" and the builder waits for whatever the header asked
// for. An hour is not a wait, it is a trap: the preview stays on its last frame and nothing
// comes of it.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
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
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

/**
 * Moves the clock on until the check holds. The demo backend answers every read after a short
 * wait, so the builder is ready after a few turns of the clock and not at once.
 */
async function advanceUntil(check: () => boolean, rounds = 400): Promise<boolean> {
  for (let i = 0; i < rounds; i += 1) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20);
    });
    if (check()) return true;
  }
  return false;
}

describe("the stack builder's busy answer", () => {
  it("Caps Retry-After so a huge header retries at 30 seconds", async () => {
    // The first reads are answered with the clock still running.
    await refreshAll();
    vi.useFakeTimers();

    const sentAt: number[] = [];
    const preview = vi.spyOn(api, "putStackPreview").mockImplementation(() => {
      sentAt.push(Date.now());
      return Promise.reject(
        new ApiFailure(503, "service_unavailable", "the daemon is busy", 3_600_000),
      );
    });

    try {
      render(
        <ToastProvider>
          <ToastHost />
          <StripSlotsProvider>
            <div className="strip-slots">
              <StripSlot kind="title" />
              <StripSlot kind="actions" />
            </div>
            <EffectsView />
          </StripSlotsProvider>
        </ToastProvider>,
      );
      // The builder reads the stack and sends its first preview. The demo answers every read
      // after a short wait, so the clock is moved on until the preview is out.
      expect(await advanceUntil(() => sentAt.length > 0)).toBe(true);
      expect(screen.getByTestId("stack-builder")).toBeTruthy();
      const first = sentAt[0]!;
      // The wait the builder chose has a ceiling: an hour is not a wait, it is a trap.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(Math.max(0, first + 29_999 - Date.now()));
      });
      expect(preview).toHaveBeenCalledTimes(1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1);
      });
      expect(preview).toHaveBeenCalledTimes(2);
    } finally {
      preview.mockRestore();
    }
  });
});
