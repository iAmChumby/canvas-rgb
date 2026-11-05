import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// Issue #97, section 2, on the stack builder: one busy answer, one retry.
//
// The editor's own preview already does this (EffectEditor.tsx): a run that starts now takes the
// place of the retry that was waiting, so a second busy answer replaces the pending retry instead
// of joining it. The builder did not: a change sent again while a retry was waiting left the first
// retry alive, and the second busy answer overwrote its handle. Two timers were then waiting on
// one busy answer, and one busy answer cost two requests.
//
// The daemon is replaced by a mock, so the claim here is about the builder and not about a daemon
// that happened to be busy.

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
  it("Keeps one retry waiting when a change sends again", async () => {
    // The first reads are answered with the clock still running.
    await refreshAll();
    vi.useFakeTimers();

    const sentAt: number[] = [];
    const preview = vi.spyOn(api, "putStackPreview").mockImplementation(() => {
      sentAt.push(Date.now());
      // The daemon is busy, and it asks for a second.
      return Promise.reject(
        new ApiFailure(503, "service_unavailable", "the daemon is busy", 1_000),
      );
    });

    try {
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
      // The builder reads the stack and sends its first preview.
      expect(await advanceUntil(() => sentAt.length > 0)).toBe(true);
      const first = sentAt[0]!;
      // The busy answer has landed, and one retry waits for a second from now.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(sentAt.length).toBe(1);

      // A change comes while that retry is waiting.
      act(() => {
        fireEvent.click(screen.getByRole("button", { name: /(Hide|Show) Sparkles/ }));
      });
      // The change waits out the debounce, and sends again.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(400);
      });
      expect(sentAt.length).toBe(2);
      // The second busy answer waits for a retry of its own.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(sentAt.length).toBe(2);

      // The first retry's own deadline passes. The send that went out took the place of the retry
      // that was waiting, so nothing is sent at that deadline.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(Math.max(0, first + 1_100 - Date.now()));
      });
      expect(sentAt.length).toBe(2);

      // The retry that is waiting fires once, and once only.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(Math.max(0, first + 1_500 - Date.now()));
      });
      expect(sentAt.length).toBe(3);

      // And a busy daemon is never polled: the retry that was answered busy does not ask again.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5_000);
      });
      expect(sentAt.length).toBe(3);
    } finally {
      preview.mockRestore();
    }
  });
});
