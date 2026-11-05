import { StrictMode } from "react";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { EffectEditor, type EditorTarget } from "@/views/effects/EffectEditor";
import type { EffectPreviewResponse } from "@/proto/EffectPreviewResponse";

// Issue #171: `mounted` starts true (EffectEditor.tsx), the unmount cleanup sets it false,
// and nothing sets it back to true. StrictMode mounts, runs the cleanup, then re-runs the
// effects on the same instance: `mounted` stays false and every preview answer is discarded
// at the `mounted`/`seq` guard, leaving the preview on "rendering". The editor is rendered
// inside `<StrictMode>` here, with the same mocked daemon as EffectPreviewRetry.test.tsx.
// The idle note also reads "Preview", so the assertion is on `data-preview`: only an
// accepted answer moves it to "ok"; a discarded one leaves it on "rendering".

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

afterEach(cleanup);

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.draft.mockResolvedValue({
    active: false,
    props: {},
    lease_remaining_ms: 0,
    heartbeat_ms: 0,
    lease_ms: 0,
  });
});

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

describe("the editor under StrictMode", () => {
  it("shows the preview answer instead of sitting on rendering", async () => {
    api.previewEffect.mockResolvedValue(drawn());
    const { container } = render(
      <StrictMode>
        <EffectEditor target={target()} narrow onClose={() => {}} onSaved={() => {}} />
      </StrictMode>,
    );
    // StrictMode's first pass consumes the immediate first preview, so the remounted one
    // waits for the 400 ms debounce. Wait past it and let the answer promise settle.
    await act(async () => {
      await new Promise<void>((resolve) => setTimeout(resolve, 500));
      await Promise.resolve();
    });
    expect(api.previewEffect).toHaveBeenCalled();
    expect(container.querySelector(".eff-live")?.getAttribute("data-preview")).toBe("ok");
    expect(container.querySelector(".eff-live-note")?.textContent).toBe("Preview");
  });
});
