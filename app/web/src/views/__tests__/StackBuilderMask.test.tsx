// The stack builder's mask controls (#256): the row's Add mask, the picker's six leads, and the
// mask section's effect, source, invert and amount, all against the demo backend.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import type { LayerMask } from "@/proto/LayerMask";

const feed = vi.hoisted(() => ({ listener: null as ((problem: string | null) => void) | null }));

vi.mock("@/lib/stackPreview", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/stackPreview")>();
  return {
    ...actual,
    onStackPreviewFeed: (listener: (problem: string | null) => void) => {
      feed.listener = listener;
      return () => {
        feed.listener = null;
      };
    },
  };
});

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

type PreviewCall = { at: number; epoch: string; seq: number; applied: boolean; stack?: { layers: Array<{ id: string; mask?: LayerMask }> } };

const previewCalls = () => ((globalThis as { __stackPreview?: PreviewCall[] }).__stackPreview ??= []);

async function openBuilder(stack: string, extraState?: string) {
  const state = extraState ? `complete,stack,html,${extraState}` : "complete,stack,html";
  window.history.replaceState({}, "", `/effects/?stack=${encodeURIComponent(stack)}&state=${state}`);
  await refreshAll();
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
  await screen.findByTestId("stack-builder");
}

/** Waits until the newest preview the builder sent carries a mask that passes `check`. */
async function waitForMask(layerId: string, check: (mask: LayerMask | undefined) => boolean) {
  await waitFor(() => {
    const mask = previewCalls()
      .at(-1)
      ?.stack?.layers.find((layer) => layer.id === layerId)?.mask;
    expect(check(mask)).toBe(true);
  });
  return previewCalls().at(-1)?.stack?.layers.find((layer) => layer.id === layerId)?.mask;
}

describe("the stack builder's masks", () => {
  it("adds a mask from a layer row, leading with the six that read best", async () => {
    await openBuilder("Sunset glow");
    await screen.findByRole("heading", { name: "Sunset glow" });
    await waitFor(() => expect(screen.getAllByRole("listitem").length).toBeGreaterThanOrEqual(3));
    previewCalls().length = 0;

    fireEvent.click(screen.getByRole("button", { name: "Add a mask to Sparkles" }));
    const list = await screen.findByRole("listbox", { name: "Effects to use as a mask" });
    const names = within(list)
      .getAllByRole("option")
      .map((option) => option.querySelector(".rd-add-name")?.textContent ?? "");
    expect(names.slice(0, 6)).toEqual(["Fire", "Rain", "Spiral", "Aurora", "Breathing", "Gradient"]);
    expect(within(list).getByText("Good as a mask")).toBeTruthy();
    expect(within(list).getByText("More")).toBeTruthy();

    fireEvent.click(within(list).getByRole("option", { name: /^Fire/ }));
    const remove = await screen.findByRole("button", { name: "Remove the mask from Sparkles" });
    expect(remove).toBeTruthy();
    // The row shows the mask's own small thumbnail beside the layer's.
    const row = document.querySelector('.layer-row[data-layer-id="l3"]');
    expect(row?.querySelector(".layer-mask-thumb")).not.toBeNull();
    const mask = await waitForMask("l3", (mask) => mask?.effect === "fire");
    expect(mask).toMatchObject({ effect: "fire", source: "luma", invert: false, amount: 100 });
  });

  it("edits the mask's effect, source, invert and amount, and every change reaches the preview", async () => {
    await openBuilder("Warm flame");
    await screen.findByRole("heading", { name: "Warm flame" });
    await screen.findByRole("button", { name: "Remove the mask from Warm white" });

    // The section says which effect the mask is, and its own controls sit under their heading.
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Mask effect" }).textContent).toContain("Fire"),
    );
    expect(screen.getByRole("heading", { name: "Fire settings" })).toBeTruthy();
    previewCalls().length = 0;

    fireEvent.click(screen.getByRole("radio", { name: "Strongest colour" }));
    await waitForMask("l1", (mask) => mask?.source === "alpha");

    fireEvent.click(screen.getByRole("switch", { name: "Invert the mask" }));
    await waitForMask("l1", (mask) => mask?.invert === true);

    fireEvent.keyDown(screen.getByRole("slider", { name: "Mask amount" }), { key: "Home" });
    await waitForMask("l1", (mask) => mask?.amount === 0);

    fireEvent.click(screen.getByRole("combobox", { name: "Mask effect" }));
    fireEvent.click(await screen.findByRole("option", { name: /^Rain\b/ }));
    await waitForMask("l1", (mask) => mask?.effect === "rain");
  });

  it("removes a mask from the layer's panel", async () => {
    await openBuilder("Warm flame");
    await screen.findByRole("button", { name: "Remove the mask from Warm white" });
    previewCalls().length = 0;

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    await waitForMask("l1", (mask) => mask === undefined);
    expect(screen.queryByRole("button", { name: "Remove the mask from Warm white" })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Add mask" }).length).toBeGreaterThan(0);
  });

  it("marks a mask edit unsaved, and Revert puts the mask back", async () => {
    await openBuilder("Warm flame");
    await screen.findByRole("heading", { name: "Warm flame" });
    await waitFor(() =>
      expect(screen.getByRole("slider", { name: "Mask amount" }).getAttribute("aria-valuenow")).toBe("100"),
    );
    expect(screen.queryByText("Unsaved")).toBeNull();

    fireEvent.keyDown(screen.getByRole("slider", { name: "Mask amount" }), { key: "Home" });
    await waitFor(() =>
      expect(screen.getByRole("slider", { name: "Mask amount" }).getAttribute("aria-valuenow")).toBe("0"),
    );
    expect(screen.getByText("Unsaved")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Revert" }));
    await waitFor(() => expect(screen.queryByText("Unsaved")).toBeNull());
    expect(screen.getByRole("slider", { name: "Mask amount" }).getAttribute("aria-valuenow")).toBe("100");
  });

  it("shows the HTML-cap refusal beside the mask Select, not at the bottom of the panel", async () => {
    await openBuilder("Sunset glow");
    await screen.findByRole("heading", { name: "Sunset glow" });
    await waitFor(() => expect(screen.getAllByRole("listitem").length).toBeGreaterThanOrEqual(3));
    // Two HTML layers take both slots.
    for (const name of ["Sample HTML", "Ink Drop"]) {
      await waitFor(() => expect(screen.queryByRole("listbox", { name: "Effects to add" })).toBeNull());
      fireEvent.click(screen.getByRole("button", { name: /Add layer/ }));
      fireEvent.click(await screen.findByRole("option", { name: new RegExp(name) }));
      await waitFor(() => expect(screen.getAllByRole("listitem").length).toBeGreaterThanOrEqual(4));
    }
    // The bottom layer is not HTML; give it a Fire mask, then ask for an HTML mask.
    await waitFor(() => expect(screen.queryByRole("listbox", { name: "Effects to add" })).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "Add a mask to Sunset" }));
    const masks = await screen.findByRole("listbox", { name: "Effects to use as a mask" });
    fireEvent.click(within(masks).getByRole("option", { name: /^Fire/ }));
    await screen.findByRole("combobox", { name: "Mask effect" });
    fireEvent.click(screen.getByRole("combobox", { name: "Mask effect" }));
    const maskOptions = await screen.findByRole("listbox", { name: "Mask effect" });
    fireEvent.click(within(maskOptions).getByRole("option", { name: /Wet Smear/ }));

    const problem = await screen.findByText("A stack holds 2 HTML layers");
    expect(problem.getAttribute("role")).toBe("alert");
    const mask = document.querySelector(".rd-mask");
    expect(mask?.contains(problem)).toBe(true);
    expect(problem.closest(".rd-layer-edit .rd-mask")).not.toBeNull();

    // The refusal belongs to the layer it was said on: picking another layer clears it (#256).
    const other = document.querySelector<HTMLElement>('.layer-row[data-layer-id="l1"]');
    expect(other).not.toBeNull();
    fireEvent.click(other!);
    await waitFor(() => expect(screen.queryByText("A stack holds 2 HTML layers")).toBeNull());
  });

  it("moves through the mask picker with the arrow keys and chooses with Enter", async () => {
    await openBuilder("Warm flame");
    await screen.findByRole("heading", { name: "Warm flame" });
    // Change the mask to Rain first, so "focus starts on the current mask" is not just the first row.
    await screen.findByRole("combobox", { name: "Mask effect" });
    fireEvent.click(screen.getByRole("combobox", { name: "Mask effect" }));
    const options = await screen.findByRole("listbox", { name: "Mask effect" });
    fireEvent.click(within(options).getByRole("option", { name: /^Rain\b/ }));
    await waitForMask("l1", (mask) => mask?.effect === "rain");

    fireEvent.click(screen.getByRole("button", { name: "Change the mask of Warm white" }));
    const list = await screen.findByRole("listbox", { name: "Effects to use as a mask" });
    expect(document.activeElement).toBe(list);
    expect(list.getAttribute("aria-activedescendant")).toBe("mask-option-rain");

    fireEvent.keyDown(list, { key: "ArrowDown" });
    expect(list.getAttribute("aria-activedescendant")).toBe("mask-option-spiral");
    fireEvent.keyDown(list, { key: "ArrowUp" });
    expect(list.getAttribute("aria-activedescendant")).toBe("mask-option-rain");
    fireEvent.keyDown(list, { key: "End" });
    const last = within(list).getAllByRole("option").at(-1);
    expect(list.getAttribute("aria-activedescendant")).toBe(last?.id);
    fireEvent.keyDown(list, { key: "Home" });
    expect(list.getAttribute("aria-activedescendant")).toBe("mask-option-fire");

    fireEvent.keyDown(list, { key: "ArrowDown" });
    fireEvent.keyDown(list, { key: "ArrowDown" });
    expect(list.getAttribute("aria-activedescendant")).toBe("mask-option-spiral");
    fireEvent.keyDown(list, { key: "Enter" });
    await waitForMask("l1", (mask) => mask?.effect === "spiral");

    // Space chooses too.
    fireEvent.click(screen.getByRole("button", { name: "Change the mask of Warm white" }));
    const again = await screen.findByRole("listbox", { name: "Effects to use as a mask" });
    expect(again.getAttribute("aria-activedescendant")).toBe("mask-option-spiral");
    fireEvent.keyDown(again, { key: "ArrowDown" });
    fireEvent.keyDown(again, { key: " " });
    await waitForMask("l1", (mask) => mask?.effect === "aurora");
  });

  it("gives a layer without a mask the Add mask key in its panel", async () => {
    await openBuilder("Sunset glow");
    await screen.findByRole("heading", { name: "Sunset glow" });
    await waitFor(() => expect(screen.getAllByRole("listitem").length).toBeGreaterThanOrEqual(3));
    // The picked layer is the top one, which has no mask in the demo.
    expect(screen.getByText(/A mask shapes where this layer shows/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Add mask" }));
    expect(await screen.findByRole("listbox", { name: "Effects to use as a mask" })).toBeTruthy();
  });
});
