import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { refreshAll } from "@/lib/store";
import type { StacksResponse } from "@/proto/StacksResponse";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The stack builder against the demo backend (D95.5): the seeded Sunset glow, its layers, the
// preview PUTs, Try on the lights and Save. No daemon, no socket: the demo answers every route.

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

type PreviewCall = { at: number; epoch: string; seq: number; applied: boolean; stack?: { layers: Array<{ id: string; opacity: number }> } };
type SaveCall = { at: number; id: string; layers: number };
type DraftCall = { method: string; stack?: { id: string } };

const previewCalls = () => ((globalThis as { __stackPreview?: PreviewCall[] }).__stackPreview ??= []);
const saveCalls = () => ((globalThis as { __stackSaves?: SaveCall[] }).__stackSaves ??= []);
const draftCalls = () => ((globalThis as { __draft?: DraftCall[] }).__draft ??= []);

async function renderEffects(query: string) {
  window.history.replaceState({}, "", `/effects/${query}`);
  await refreshAll();
  return render(
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
}

async function openBuilder() {
  const view = await renderEffects("?stack=Sunset%20glow&state=complete,stack");
  await screen.findByTestId("stack-builder");
  await screen.findByRole("heading", { name: "Sunset glow" });
  return view;
}

/** The rows of the layer list, top first. */
function layerNames(): string[] {
  return screen
    .getAllByRole("listitem")
    .map((row) => row.querySelector(".layer-name")?.textContent ?? "")
    .filter(Boolean);
}

describe("the stack builder", () => {
  it("opens the saved stack with its layers, top first", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toEqual(["Sparkles", "Heat", "Sunset"]));
    expect(screen.getByText("3 / 8")).toBeTruthy();
    // The strip carries the stack's name and its context.
    expect(screen.getByRole("heading", { name: "Sunset glow" })).toBeTruthy();
    expect(screen.getByText("Stack · 3 layers")).toBeTruthy();
  });

  it("adds a layer from the library, and the preview is sent after the change", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    previewCalls().length = 0;
    fireEvent.click(screen.getByRole("button", { name: /Add layer/ }));
    const solid = await screen.findByRole("option", { name: /Solid/ });
    fireEvent.click(solid);
    await waitFor(() => expect(layerNames()).toHaveLength(4));
    await waitFor(() => expect(previewCalls().some((call) => call.stack?.layers.length === 4)).toBe(true));
    const last = previewCalls().at(-1);
    expect(last?.applied).toBe(true);
    expect(last?.seq).toBeGreaterThan(0);
  });

  it("asks before leaving with unsaved changes", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    fireEvent.click(screen.getByRole("button", { name: /Add layer/ }));
    fireEvent.click(await screen.findByRole("option", { name: /Solid/ }));
    await waitFor(() => expect(layerNames()).toHaveLength(4));
    expect(screen.getByText("Unsaved")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    const prompt = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    expect(prompt).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Keep editing" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull());
  });

  it("tries the stack on the lights and stops it", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    draftCalls().length = 0;
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await screen.findByText("Draft on the lights");
    const put = draftCalls().find((call) => call.method === "PUT");
    expect(put?.stack?.id).toBe("stack:Sunset glow");
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(screen.queryByText("Draft on the lights")).toBeNull());
    expect(draftCalls().some((call) => call.method === "DELETE")).toBe(true);
  });

  it("saves the stack, then activates it", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    saveCalls().length = 0;
    // A layer change first, so the Save has something to write.
    fireEvent.click(screen.getByRole("button", { name: /Add layer/ }));
    fireEvent.click(await screen.findByRole("option", { name: /Solid/ }));
    await waitFor(() => expect(layerNames()).toHaveLength(4));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saveCalls()).toHaveLength(1));
    expect(saveCalls()[0]?.id).toBe("stack:Sunset glow");
    expect(saveCalls()[0]?.layers).toBe(4);
    await waitFor(() => expect(screen.queryByText("Unsaved")).toBeNull());
  });

  it("replaces a taken name in one write, and a failed write keeps the other stack", async () => {
    // What the saved stack holds before this test: the file's tests share one demo backend.
    const before = await api.stacks();
    const layersBefore = before.stacks.find((entry) => entry.id === "stack:Sunset glow")?.layers ?? null;
    // The stack list lands after the first Save, as when the name was taken by another window:
    // the client cannot refuse the name, the daemon's 409 does, and the Replace prompt appears
    // with the list now loaded.
    let release: ((value: StacksResponse) => void) | null = null;
    const listSpy = vi.spyOn(api, "stacks").mockImplementation(
      () =>
        new Promise<StacksResponse>((resolve) => {
          release = resolve;
        }),
    );
    const putSpy = vi.spyOn(api, "putStack");
    const deleteSpy = vi.spyOn(api, "deleteStack");
    try {
      await renderEffects("?stack=new&state=complete,stack");
      await screen.findByTestId("stack-builder");
      fireEvent.click(screen.getByRole("button", { name: "Add the first layer" }));
      fireEvent.click(await screen.findByRole("option", { name: /Solid/ }));
      await waitFor(() => expect(layerNames()).toHaveLength(1));

      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      const naming = await screen.findByRole("dialog", { name: "Name the stack" });
      fireEvent.change(within(naming).getByRole("textbox", { name: "Stack name" }), {
        target: { value: "sunset glow" },
      });
      fireEvent.click(within(naming).getByRole("button", { name: "Save" }));
      const clash = await screen.findByRole("alert");
      expect(clash.textContent).toContain("Replace it with this stack?");
      expect(clash.textContent).toContain("its presets and favourite stay");

      await act(async () => {
        release?.({
          stacks: [{ id: "stack:Sunset glow", name: "Sunset glow", layers: 3, html_layers: 0, missing: [] }],
        });
        await Promise.resolve();
      });

      // The write fails: nothing was deleted, the other stack is untouched, and the error shows.
      putSpy.mockRejectedValueOnce(new ApiFailure(500, "internal", "the store is read-only"));
      fireEvent.click(screen.getByRole("button", { name: "Replace" }));
      await waitFor(() => expect(screen.getByText(/read-only/)).toBeTruthy());
      expect(deleteSpy).not.toHaveBeenCalled();
      expect(putSpy.mock.calls.at(-1)?.[0]).toBe("stack:Sunset glow");
      listSpy.mockRestore();
      const still = await api.stacks();
      expect(still.stacks.find((entry) => entry.id === "stack:Sunset glow")?.layers).toBe(layersBefore);
    } finally {
      listSpy.mockRestore();
      putSpy.mockRestore();
      deleteSpy.mockRestore();
    }
  });

  it("sends 400 ms after the last change of a burst", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toContain("Sparkles"));
    // The first draft's own send has to be recorded before the clock is faked and the list cleared.
    await waitFor(() => expect(previewCalls().length).toBeGreaterThan(0));
    previewCalls().length = 0;
    vi.useFakeTimers();
    try {
      const toggle = () => screen.getByRole("button", { name: /(Hide|Show) Sparkles/ });
      act(() => {
        fireEvent.click(toggle());
      });
      act(() => {
        vi.advanceTimersByTime(300);
      });
      act(() => {
        fireEvent.click(toggle());
      });
      // Nothing at t=649: the last change was at 300, so the send is 400 ms after it.
      await act(async () => {
        vi.advanceTimersByTime(349);
        await Promise.resolve();
      });
      expect(previewCalls()).toHaveLength(0);
      await act(async () => {
        vi.advanceTimersByTime(101);
        await Promise.resolve();
      });
      expect(previewCalls()).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("sends every 400 ms while changes keep coming", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toContain("Sparkles"));
    // The first draft's own send has to be recorded before the clock is faked and the list cleared.
    await waitFor(() => expect(previewCalls().length).toBeGreaterThan(0));
    previewCalls().length = 0;
    vi.useFakeTimers();
    try {
      const start = Date.now();
      const toggle = () => screen.getByRole("button", { name: /(Hide|Show) Sparkles/ });
      for (let at = 0; at < 900; at += 100) {
        act(() => {
          fireEvent.click(toggle());
        });
        await act(async () => {
          vi.advanceTimersByTime(100);
          await Promise.resolve();
        });
      }
      const sent = previewCalls().map((call) => call.at - start);
      expect(sent.length).toBeGreaterThanOrEqual(2);
      // One send about every 400 ms of the drag, not one per change and not one only at the end.
      expect(sent[0]).toBeGreaterThanOrEqual(350);
      expect(sent[0]).toBeLessThanOrEqual(550);
      const gap = sent[1]! - sent[0]!;
      expect(gap).toBeGreaterThanOrEqual(350);
      expect(gap).toBeLessThanOrEqual(500);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not resend a preview the daemon answered applied: false", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toContain("Sparkles"));
    const spy = vi.spyOn(api, "putStackPreview").mockResolvedValueOnce({
      applied: false,
      epoch: "0000000000000000",
      seq: 0,
      layers: [],
      running: false,
    });
    try {
      vi.useFakeTimers();
      act(() => {
        fireEvent.click(screen.getByRole("button", { name: /(Hide|Show) Sparkles/ }));
      });
      await act(async () => {
        vi.advanceTimersByTime(500);
        await Promise.resolve();
      });
      expect(spy).toHaveBeenCalledTimes(1);
      // The superseded answer is the end of that change: no retry and no resend.
      await act(async () => {
        vi.advanceTimersByTime(5_000);
        await Promise.resolve();
      });
      expect(spy).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
      spy.mockRestore();
    }
  });

  it("shows the daemon's validation answer for a refused Try or preview", async () => {
    await openBuilder();
    await waitFor(() => expect(layerNames()).toContain("Sparkles"));
    const draftSpy = vi.spyOn(api, "putDraft").mockRejectedValueOnce(
      new ApiFailure(422, "validation", "a stack holds at most 2 HTML layers"),
    );
    try {
      fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
      await waitFor(() =>
        expect(screen.getByText(/Try on the lights failed: a stack holds at most 2 HTML layers/)).toBeTruthy(),
      );
    } finally {
      draftSpy.mockRestore();
    }
    const previewSpy = vi.spyOn(api, "putStackPreview").mockRejectedValueOnce(
      new ApiFailure(422, "validation", "layer l1: opacity must be 0..=100"),
    );
    try {
      fireEvent.click(screen.getByRole("button", { name: /(Hide|Show) Sparkles/ }));
      await waitFor(() =>
        expect(screen.getByText(/The preview stopped: layer l1: opacity must be 0..=100/)).toBeTruthy(),
      );
    } finally {
      previewSpy.mockRestore();
    }
  });

  it("opens the builder from a stack's tile in the library", async () => {
    await renderEffects("?state=complete,stack");
    const card = await screen.findByRole("button", { name: "Edit the stack Sunset glow" });
    fireEvent.click(card);
    await screen.findByTestId("stack-builder");
    await screen.findByRole("heading", { name: "Sunset glow" });
  });

  it("opens a new stack straight from the header's New effect key", async () => {
    await renderEffects("?state=complete,stack");
    fireEvent.click(await screen.findByRole("button", { name: "New effect" }));
    await screen.findByTestId("stack-builder");
    expect(screen.getByRole("heading", { name: "New stack" })).toBeTruthy();
    // There is no one-item menu any more, and the two HTML ways live under More > Advanced.
    expect(document.querySelector(".eff-new-menu")).toBeNull();
    expect(screen.queryByText("Blank HTML file")).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy an effect…" })).toBeNull();
  });

  it("starts a new stack empty, with Save and Try held back", async () => {
    await renderEffects("?stack=new&state=complete,stack");
    await screen.findByTestId("stack-builder");
    expect(screen.getByText("Nothing to show yet")).toBeTruthy();
    expect(screen.getByText("The bottom layer is painted first.")).toBeTruthy();
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toHaveProperty("disabled", true);
    const tryOn = screen.getByRole("button", { name: "Try on the lights" });
    expect(tryOn).toHaveProperty("disabled", true);
    fireEvent.click(screen.getByRole("button", { name: "Add the first layer" }));
    await screen.findByRole("dialog", { name: "Add a layer" });
  });

  it("changes the picked layer's opacity, and the preview carries it", async () => {
    await openBuilder();
    const row = screen.getByText("Sparkles").closest("li");
    expect(row).not.toBeNull();
    if (row) fireEvent.click(row);
    previewCalls().length = 0;
    const slider = screen.getByRole("slider", { name: "Opacity" });
    fireEvent.keyDown(slider, { key: "ArrowLeft" });
    // The PUT that carries the new value, not a preview that was already on its way.
    await waitFor(() => {
      const sparkles = previewCalls().at(-1)?.stack?.layers.find((layer) => layer.id === "l3");
      expect(sparkles?.opacity).toBeLessThan(80);
    });
  });
});
