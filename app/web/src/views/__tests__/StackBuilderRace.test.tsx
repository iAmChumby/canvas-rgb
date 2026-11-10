import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The Add layer race, on its own demo state: `?state=complete,stack,stock` has the stock HTML
// effects, and a fresh file gets a fresh demo store (the store is read from the first URL).

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack,stock");
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
  Range.prototype.getClientRects = (() =>
    ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList) as typeof Range.prototype.getClientRects;
  Range.prototype.getBoundingClientRect = (() =>
    ({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) }) as DOMRect) as typeof Range.prototype.getBoundingClientRect;
});

afterEach(cleanup);

type PreviewCall = { stack?: { layers: Array<{ id: string }> } };

const previewCalls = () => ((globalThis as { __stackPreview?: PreviewCall[] }).__stackPreview ??= []);

async function renderBuilder() {
  await refreshAll();
  return render(
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
}

function layerNames(): string[] {
  return screen
    .getAllByRole("listitem")
    .map((row) => row.querySelector(".layer-name")?.textContent ?? "")
    .filter(Boolean);
}

describe("adding a layer while its effect is still loading", () => {
  it("adds one layer when one effect row is clicked twice quickly", async () => {
    await renderBuilder();
    await screen.findByTestId("stack-builder");
    await waitFor(() => expect(layerNames()).toContain("Sparkles"));
    const before = layerNames().length;
    // The effect read is held, so both clicks land before the first add finishes.
    const realEffect = api.effect;
    const ink = await realEffect("html:ink-drop");
    const pending: Array<(detail: typeof ink) => void> = [];
    const spy = vi.spyOn(api, "effect").mockImplementation((id) =>
      id.startsWith("html:") ? new Promise((resolve) => pending.push(resolve)) : realEffect(id),
    );
    try {
      const row = await screen.findByRole("button", { name: "Add Ink Drop" });
      await act(async () => {
        fireEvent.click(row);
        fireEvent.click(row);
        await Promise.resolve();
        await Promise.resolve();
      });
      // The second click is the same add: one read, not two.
      expect(pending).toHaveLength(1);
      await act(async () => {
        pending[0]?.(ink);
        await Promise.resolve();
      });
      await waitFor(() => expect(layerNames()).toHaveLength(before + 1));
      await waitFor(() => {
        const layers = previewCalls().at(-1)?.stack?.layers ?? [];
        expect(layers.length).toBe(before + 1);
      });
      const ids = (previewCalls().at(-1)?.stack?.layers ?? []).map((layer) => layer.id);
      expect(new Set(ids).size).toBe(ids.length);
    } finally {
      spy.mockRestore();
    }
  });

  it("adds both when two different effects are clicked while the first is read", async () => {
    await renderBuilder();
    await screen.findByTestId("stack-builder");
    await waitFor(() => expect(layerNames()).toContain("Sparkles"));
    const before = layerNames().length;
    const realEffect = api.effect;
    const ink = await realEffect("html:ink-drop");
    const solid = await realEffect("solid");
    const pending: Array<{ effect: string; resolve: (detail: typeof ink) => void }> = [];
    const spy = vi.spyOn(api, "effect").mockImplementation((id) => {
      if (id === "html:ink-drop" || id === "solid") {
        return new Promise((resolve) => pending.push({ effect: id, resolve }));
      }
      return realEffect(id);
    });
    try {
      const inkRow = await screen.findByRole("button", { name: "Add Ink Drop" });
      const solidRow = await screen.findByRole("button", { name: "Add Solid" });
      await act(async () => {
        fireEvent.click(inkRow);
        fireEvent.click(solidRow);
        await Promise.resolve();
        await Promise.resolve();
      });
      // The second add waits its turn in the queue rather than being dropped.
      expect(pending.map((entry) => entry.effect)).toEqual(["html:ink-drop"]);
      await act(async () => {
        pending[0]?.resolve(ink);
        await Promise.resolve();
        await Promise.resolve();
      });
      await waitFor(() => expect(pending.map((entry) => entry.effect)).toEqual(["html:ink-drop", "solid"]));
      await act(async () => {
        pending[1]?.resolve(solid);
        await Promise.resolve();
      });
      await waitFor(() => expect(layerNames()).toHaveLength(before + 2));
      const ids = (previewCalls().at(-1)?.stack?.layers ?? []).map((layer) => layer.id);
      expect(new Set(ids).size).toBe(ids.length);
    } finally {
      spy.mockRestore();
    }
  });
});
