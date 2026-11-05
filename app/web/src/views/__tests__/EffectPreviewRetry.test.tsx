import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiFailure } from "@/lib/errors";
import { EffectEditor, type EditorTarget } from "@/views/effects/EffectEditor";
import type { EffectPreviewRequest } from "@/proto/EffectPreviewRequest";
import type { EffectPreviewResponse } from "@/proto/EffectPreviewResponse";

// Issue #97, sections 1 to 3. The editor asks the daemon to render the draft, and the daemon may
// answer "busy: try again in a moment". Three things were wrong with what the editor did with
// that answer:
//
// - the wait had no ceiling, so a header of an hour left the user sitting in "busy" (section 1);
// - a second busy answer left the first retry waiting, so one busy answer cost two requests
//   (section 2);
// - a preview that was no longer wanted was still running on the daemon (section 3).
//
// The daemon is replaced by a mock, so every claim here is about the editor and not about a
// daemon that happened to be busy.

const { api } = vi.hoisted(() => ({
  api: {
    previewEffect: vi.fn(),
    draft: vi.fn(),
    putDraft: vi.fn(),
    deleteDraft: vi.fn(),
    importEffect: vi.fn(),
    activateEffect: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
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
});

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  // No draft runs, and the editor does not ask for one.
  api.draft.mockResolvedValue({
    active: false,
    props: {},
    lease_remaining_ms: 0,
    heartbeat_ms: 0,
    lease_ms: 0,
  });
});

/** The editor's target: a file that was never saved, with one property to change. */
function target(): EditorTarget {
  return {
    id: null,
    name: "Sample",
    fileName: "sample.html",
    source: "<html><body>a draft</body></html>",
    props: {},
    schema: {
      id: "html:sample",
      name: "Sample",
      description: "",
      needs: { screen: false, audio: false },
      properties: [{ key: "glow", label: "Glow", kind: { type: "bool", default: false } }],
      presets: [],
    },
    presets: [],
    userPresets: [],
  };
}

/** The editor, open on that target. The code layer is closed, so no editor is loaded. */
function editor() {
  return render(
    <EffectEditor target={target()} narrow onClose={() => {}} onSaved={() => {}} />,
  );
}

/** The answer of a preview that drew: no errors, and a loop that is not there. */
function drawn(): EffectPreviewResponse {
  return {
    schema: {
      id: "draft:abc",
      name: "Sample",
      description: "",
      needs: { screen: false, audio: false },
      properties: [],
      presets: [],
    },
    source_hash: "abc",
    errors: [],
    warnings: [],
    thumbnail: "",
  };
}

describe("the editor's busy answer", () => {
  it("Caps Retry-After so a huge header retries at 30 seconds", async () => {
    vi.useFakeTimers();
    // The daemon says "busy, try again in an hour".
    api.previewEffect.mockRejectedValue(
      new ApiFailure(503, "service_unavailable", "the daemon is busy", 3_600_000),
    );
    editor();
    // The first preview goes out at once: the first one waits for no debounce.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Busy: the daemon is rendering other previews")).toBeTruthy();
    // The wait the editor chose has a ceiling: an hour is not a wait, it is a trap.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(29_999);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(2);
  });

  it("A second busy answer replaces the pending retry instead of adding to it", async () => {
    vi.useFakeTimers();
    // Every preview is busy, and the daemon asks for 400 ms.
    api.previewEffect.mockRejectedValue(
      new ApiFailure(503, "service_unavailable", "the daemon is busy", 400),
    );
    editor();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(1);
    // The person asks again while the first retry is still waiting.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Try again" }));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(2);
    // One busy answer, one retry: the waiting retry was replaced, not joined by a second one.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(3);
    // And a busy daemon is never polled: the retry that was answered busy does not ask again.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(3);
  });

  it("Aborts the superseded preview when a newer one starts", async () => {
    vi.useFakeTimers();
    const signals: (AbortSignal | undefined)[] = [];
    const pending: Array<{
      resolve: (answer: EffectPreviewResponse) => void;
      reject: (reason: unknown) => void;
    }> = [];
    api.previewEffect.mockImplementation(
      (_body: EffectPreviewRequest, opts?: { signal?: AbortSignal }) =>
        new Promise<EffectPreviewResponse>((resolve, reject) => {
          signals.push(opts?.signal);
          pending.push({ resolve, reject });
        }),
    );
    editor();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(1);
    // A newer draft starts while the first preview is still out.
    await act(async () => {
      fireEvent.click(screen.getByRole("switch", { name: "Glow" }));
      await vi.advanceTimersByTimeAsync(400);
    });
    expect(api.previewEffect).toHaveBeenCalledTimes(2);
    // The preview nobody waits for any more is let go of; the one that is waited for is not.
    expect(signals[0]?.aborted).toBe(true);
    expect(signals[1]?.aborted).toBe(false);
    // The newer preview answers, and its answer is the one on screen.
    await act(async () => {
      pending[1]?.resolve(drawn());
      await Promise.resolve();
    });
    expect(screen.getByText("Preview")).toBeTruthy();
    // The older preview is let go of: an abort is not an error, and no error is shown.
    await act(async () => {
      pending[0]?.reject(new DOMException("The operation was aborted.", "AbortError"));
      await Promise.resolve();
    });
    expect(screen.queryByText("Preview stopped: the daemon did not answer")).toBeNull();
    expect(screen.getByText("Preview")).toBeTruthy();
  });
});
