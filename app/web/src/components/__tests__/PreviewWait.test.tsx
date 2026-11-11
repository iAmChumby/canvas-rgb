import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LOAD_DELAY_MS } from "@/lib/loadDelay";

// Issue #296: the detail pane's preview used to be an `aria-hidden` canvas that said nothing and
// drew nothing until the daemon's thumbnail arrived — on a slow tile, nothing at all, for seconds.
// It now waits like every other canvas in the editor: it is a `role="img"` that is `aria-busy` while
// the fetch is out, after the delay it wears the skeleton tone (`data-wait`), and a short visible
// note says the picture is rendering. The note and the busy state drop when the answer lands.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const THUMB = { w: 1, h: 1, frames: 1, data: new Uint8Array(3) };

beforeEach(() => {
  vi.useFakeTimers();
  window.matchMedia ??= ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.doUnmock("@/lib/thumbs");
  vi.resetModules();
});

/** EffectPreview with a thumbnail fetch whose answer is held back until `answer` is called. */
async function mountPending() {
  let answer: (thumb: typeof THUMB | null) => void = () => undefined;
  vi.doMock("@/lib/thumbs", () => ({
    loadThumb: () =>
      new Promise((resolve) => {
        answer = resolve;
      }),
    blit: vi.fn(),
  }));
  const { EffectPreview } = await import("@/components/EffectPreview");
  const mounted = render(<EffectPreview id="x" label="Preview of Aurora" note />);
  return { ...mounted, answer };
}

describe("the detail pane's preview", () => {
  it("is a role=img that is aria-busy while the thumbnail is pending, with a visible note once the delay has passed", async () => {
    const { container } = await mountPending();
    const canvas = container.querySelector("canvas") as HTMLCanvasElement;
    expect(canvas.getAttribute("role")).toBe("img");
    expect(canvas.getAttribute("aria-label")).toBe("Preview of Aurora");
    expect(canvas.getAttribute("aria-busy")).toBe("true");
    expect(canvas.getAttribute("aria-hidden")).toBeNull();
    expect(canvas.dataset.wait).toBe("held");
    // Before the delay: the box holds its room, invisibly.
    expect(container.querySelector(".effect-preview-note")).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(LOAD_DELAY_MS);
    });
    expect(canvas.dataset.wait).toBe("shown");
    const note = container.querySelector(".effect-preview-note");
    expect(note?.textContent).toBe("Rendering…");
  });

  it("drops the busy state, the skeleton and the note when the thumbnail lands", async () => {
    const { container, answer } = await mountPending();
    const canvas = container.querySelector("canvas") as HTMLCanvasElement;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LOAD_DELAY_MS);
    });
    expect(canvas.dataset.wait).toBe("shown");

    await act(async () => {
      answer(THUMB);
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(canvas.dataset.wait).toBeUndefined();
    expect(canvas.getAttribute("aria-busy")).toBeNull();
    expect(container.querySelector(".effect-preview-note")).toBeNull();
  });
});
