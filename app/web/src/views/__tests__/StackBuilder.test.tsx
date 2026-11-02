import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
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

  it("opens the builder from a stack's tile in the library", async () => {
    await renderEffects("?state=complete,stack");
    const card = await screen.findByRole("button", { name: "Edit the stack Sunset glow" });
    fireEvent.click(card);
    await screen.findByTestId("stack-builder");
    await screen.findByRole("heading", { name: "Sunset glow" });
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
