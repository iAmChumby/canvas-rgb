import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The stack builder's states at a desktop size (D95.5.8, PR D): a read that failed, a save that
// failed, eight layers, a nested stack, a preset applied to the draft, and the loading frame.
// No daemon and no socket: the demo backend answers every route.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=stack");
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

type PreviewCall = {
  at: number;
  stack?: { layers: Array<{ id: string; effect: string; props: Record<string, { value: unknown }> }> };
};
type SaveCall = { at: number; id: string; layers: number };

const previewCalls = () => ((globalThis as { __stackPreview?: PreviewCall[] }).__stackPreview ??= []);
const saveCalls = () => ((globalThis as { __stackSaves?: SaveCall[] }).__stackSaves ??= []);

async function renderEffects(query: string) {
  window.history.replaceState({}, "", `/effects/${query}`);
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

async function openBuilder(query = "?stack=Sunset%20glow&state=complete,stack") {
  await renderEffects(query);
  await screen.findByTestId("stack-builder");
  await waitFor(() => expect(layerNames().length).toBeGreaterThan(0));
}

describe("the stack builder's states", () => {
  it("shows the loading frame in the builder's shape before the stack arrives", async () => {
    await renderEffects("?stack=Sunset%20glow&state=complete,stack,slow");
    await screen.findByTestId("stack-builder");
    // The frame keeps the builder's shape while the read is out.
    expect(document.querySelector('.rd-stack-frame[data-state="loading"]')).toBeTruthy();
    expect(document.querySelector(".rd-stack-shape")).toBeTruthy();
    await waitFor(() => expect(layerNames()).toContain("Sparkles"), { timeout: 4000 });
    expect(document.querySelector('.rd-stack-frame[data-state="loading"]')).toBeNull();
  });

  it("shows the read error in the layer column, holds Save back, and reads again", async () => {
    const read = vi.spyOn(api, "stack").mockRejectedValueOnce(
      new ApiFailure(500, "internal", "could not read stacks.json: the file is locked"),
    );
    try {
      await renderEffects("?stack=Sunset%20glow&state=complete,stack");
      expect(await screen.findByText("The stack could not be read")).toBeTruthy();
      expect(document.querySelector(".rd-stack-main")?.textContent).toContain("The stack could not be read");
      // A failed read never becomes an empty save.
      expect(screen.getByRole("button", { name: "Save" })).toHaveProperty("disabled", true);
      fireEvent.click(screen.getByRole("button", { name: "Try again" }));
      await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
      await waitFor(() => expect(layerNames()).toContain("Sparkles"));
      expect(screen.getByRole("button", { name: "Save" })).toHaveProperty("disabled", false);
    } finally {
      read.mockRestore();
    }
  });

  it("shows a save error in the layer column with Try again, and keeps the draft", async () => {
    await openBuilder();
    fireEvent.click(await screen.findByRole("button", { name: "Add Solid" }));
    await waitFor(() => expect(layerNames()).toHaveLength(4));
    const write = vi.spyOn(api, "putStack").mockRejectedValueOnce(
      new ApiFailure(500, "internal", "could not write stacks.json: the file is locked"),
    );
    try {
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      expect(await screen.findByText("The stack could not be saved")).toBeTruthy();
      expect(screen.getByText(/the file is locked/)).toBeTruthy();
      expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
      // The draft is still there, with its four layers.
      expect(layerNames()).toHaveLength(4);
    } finally {
      write.mockRestore();
    }
  });

  it("says a saved stack did not start, not that it could not be saved, and starts it again", async () => {
    await openBuilder();
    fireEvent.click(await screen.findByRole("button", { name: "Add Solid" }));
    await waitFor(() => expect(layerNames()).toHaveLength(4));
    // The write lands; only the start is refused (#235).
    const start = vi.spyOn(api, "activateEffect").mockRejectedValueOnce(
      new ApiFailure(503, "service_unavailable", "the effect could not start: the engine is busy"),
    );
    try {
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      expect(await screen.findByText("Saved, but it did not start")).toBeTruthy();
      expect(screen.queryByText("The stack could not be saved")).toBeNull();
      expect(screen.getByText(/the engine is busy/)).toBeTruthy();
      // The stack is saved: the draft stays at four layers and Save has nothing left to write.
      expect(layerNames()).toHaveLength(4);
      // Start again sends the activation only, to the id that was saved.
      const write = vi.spyOn(api, "putStack");
      fireEvent.click(screen.getByRole("button", { name: "Start again" }));
      await waitFor(() => expect(screen.queryByText("Saved, but it did not start")).toBeNull());
      expect(start).toHaveBeenCalledTimes(2);
      expect(start.mock.calls[1]?.[0]).toBe("stack:Sunset glow");
      expect(write).not.toHaveBeenCalled();
      write.mockRestore();
    } finally {
      start.mockRestore();
    }
  });

  it("shows eight layers as 8 / 8, with every tile disabled and the cap as its reason", async () => {
    await renderEffects("?stack=Eight%20layers&state=complete,stack");
    await waitFor(() => expect(screen.getByText("8 / 8")).toBeTruthy());
    const add = screen.getByRole("button", { name: "Add Solid" });
    expect(add).toHaveProperty("disabled", true);
    expect(add.getAttribute("title")).toContain("A stack holds 8 layers");
  });

  it("applies a preset to the draft and the preview, and saves only with Save", async () => {
    await openBuilder();
    saveCalls().length = 0;
    previewCalls().length = 0;
    // The preset chips arrive through their own read after the stack's, so wait for the chip.
    fireEvent.click(await screen.findByRole("button", { name: "Apply preset Embers" }));
    // The change is unsaved, and nothing is written.
    expect(await screen.findByText("Unsaved")).toBeTruthy();
    expect(saveCalls()).toHaveLength(0);
    // The preview carries the preset's values: Embers sets Heat to 90 and Sunset's first
    // colour to #8a0303.
    await waitFor(() => {
      const last = previewCalls().at(-1);
      const heat = last?.stack?.layers.find((layer) => layer.id === "l2");
      expect(heat?.props.intensity?.value).toBe(90);
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saveCalls()).toHaveLength(1));
    const saved = await api.stack("Sunset glow");
    const heat = saved.stack.layers.find((layer) => layer.id === "l2");
    expect(heat?.props.intensity).toEqual({ type: "number", value: 90 });
  });

  it("filters the library's tiles by the search, and clears again", async () => {
    await openBuilder();
    const search = screen.getByRole("searchbox", { name: "Search the library" });
    const names = () =>
      [...document.querySelectorAll<HTMLElement>(".rd-library [data-lib-tile]")]
        .map((tile) => tile.dataset.effect)
        .filter((id) => id !== undefined);
    const before = names().length;
    fireEvent.change(search, { target: { value: "fire" } });
    // Fire leads, and the tiles that do not match its words are gone.
    await waitFor(() => expect(names()[0]).toBe("fire"));
    expect(names().length).toBeLessThan(before);
    fireEvent.change(search, { target: { value: "" } });
    await waitFor(() => expect(names()).toHaveLength(before));
  });

  it("closes the area picker on Escape and puts focus back on Change area", async () => {
    await openBuilder();
    const row = screen.getByText("Heat").closest("li");
    if (row) fireEvent.click(row);
    const change = screen.getByRole("button", { name: "Change area" });
    fireEvent.click(change);
    await screen.findByRole("dialog", { name: "Area for Heat" });
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Area for Heat" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(change));
  });
});
