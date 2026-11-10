import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The picked layer's inspector sections (#257 phase 4, spec §3 and §6): its effect and blend, its
// area, the effect's own properties, its mask, and the stack's presets. A number is a readout
// beside its slider: a click turns it into the field, Enter commits one undo step, Escape puts the
// value back and leaves.

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

/** The rows of the layer list, top first. */
function layerNames(): string[] {
  return screen
    .getAllByRole("listitem")
    .map((row) => row.querySelector(".layer-name")?.textContent ?? "")
    .filter(Boolean);
}

async function openBuilder() {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
  await refreshAll();
  const view = render(
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
  await screen.findByRole("heading", { name: "Sunset glow" });
  await waitFor(() => expect(layerNames()).toHaveLength(3));
  return view;
}

function pick(name: string) {
  const row = screen.getByText(name).closest("li");
  expect(row).not.toBeNull();
  if (row) fireEvent.click(row);
}

describe("the inspector's sections", () => {
  it("splits the picked layer into effect and blend, area, properties, mask and presets", async () => {
    await openBuilder();
    pick("Heat");
    await screen.findByRole("slider", { name: "Intensity" });
    await screen.findByRole("button", { name: "Apply preset Embers" });
    for (const title of ["Effect and blend", "Area", "Properties", "Mask", "Presets"]) {
      expect(screen.getByRole("heading", { name: title })).toBeTruthy();
    }
    // The controls live inside their own section, not in one undivided form.
    const blend = screen.getByRole("heading", { name: "Effect and blend" }).closest("section")!;
    expect(blend.textContent).toContain("Blend");
    expect(blend.textContent).toContain("Opacity");
    const area = screen.getByRole("heading", { name: "Area" }).closest("section")!;
    expect(area.textContent).toContain("Change area");
    const properties = screen.getByRole("heading", { name: "Properties" }).closest("section")!;
    expect(properties.textContent).toContain("Intensity");
    const mask = screen.getByRole("heading", { name: "Mask" }).closest("section")!;
    expect(mask.textContent).toContain("Add mask");
    const presets = screen.getByRole("heading", { name: "Presets" }).closest("section")!;
    expect(presets.textContent).toContain("Embers");
  });

  it("folds a section with its heading key, and opens it again", async () => {
    await openBuilder();
    pick("Heat");
    await screen.findByRole("slider", { name: "Intensity" });
    const key = screen.getByRole("button", { name: "Area" });
    const body = key.closest("section")!.querySelector(".rd-section-body")!;
    expect(key.getAttribute("aria-expanded")).toBe("true");
    expect(body.hasAttribute("hidden")).toBe(false);
    fireEvent.click(key);
    expect(key.getAttribute("aria-expanded")).toBe("false");
    expect(body.hasAttribute("hidden")).toBe(true);
    fireEvent.click(key);
    expect(body.hasAttribute("hidden")).toBe(false);
    expect(screen.getByRole("button", { name: "Change area" })).toBeTruthy();
  });

  it("keeps the slider, and its figure opens the field: Enter commits one undo step, Escape reverts", async () => {
    await openBuilder();
    pick("Heat");
    const valueNow = () => screen.getByRole("slider", { name: "Intensity" }).getAttribute("aria-valuenow");
    await waitFor(() => expect(valueNow()).toBe("72"));
    // Nothing but the figure is drawn until it is clicked.
    expect(screen.queryByRole("textbox", { name: "Intensity value" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /^Edit Intensity, \d+$/ }));
    const field = screen.getByRole("textbox", { name: "Intensity value" });
    fireEvent.change(field, { target: { value: "40" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(valueNow()).toBe("40"));
    expect(screen.queryByRole("textbox", { name: "Intensity value" })).toBeNull();
    // One gesture, one step: an undo puts the typed change back, and a redo brings it again.
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(valueNow()).toBe("72"));
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true, shiftKey: true }));
    await waitFor(() => expect(valueNow()).toBe("40"));

    // Escape leaves the value and adds no step: one undo takes the typed change back.
    fireEvent.click(screen.getByRole("button", { name: /^Edit Intensity, \d+$/ }));
    const again = screen.getByRole("textbox", { name: "Intensity value" });
    fireEvent.change(again, { target: { value: "11" } });
    fireEvent.keyDown(again, { key: "Escape" });
    expect(valueNow()).toBe("40");
    expect(screen.queryByRole("textbox", { name: "Intensity value" })).toBeNull();
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(valueNow()).toBe("72"));
  });
  it("names the readout with its value, and a held arrow in the open field is one undo step", async () => {
    await openBuilder();
    pick("Heat");
    const valueNow = () => screen.getByRole("slider", { name: "Intensity" }).getAttribute("aria-valuenow");
    await waitFor(() => expect(valueNow()).toBe("72"));
    const readout = screen.getByRole("button", { name: "Edit Intensity, 72" });
    fireEvent.click(readout);
    // Three presses of the arrow, as a held key repeats: the field stays open and each press steps.
    for (let press = 0; press < 3; press += 1) {
      fireEvent.keyDown(screen.getByRole("textbox", { name: "Intensity value" }), { key: "ArrowUp" });
    }
    await waitFor(() => expect(valueNow()).toBe("75"));
    expect(screen.getByRole("textbox", { name: "Intensity value" })).toBeTruthy();
    // The presses ran into one run, as the number field's did: one undo puts all three back.
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(valueNow()).toBe("72"));
  });

  it("hands focus back to the readout after Enter and after Escape", async () => {
    await openBuilder();
    pick("Heat");
    await waitFor(() =>
      expect(screen.getByRole("slider", { name: "Intensity" }).getAttribute("aria-valuenow")).toBe("72"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Edit Intensity, 72" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Intensity value" }), { target: { value: "50" } });
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Intensity value" }), { key: "Enter" });
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Edit Intensity, 50" })),
    );
    fireEvent.click(screen.getByRole("button", { name: "Edit Intensity, 50" }));
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Intensity value" }), { key: "Escape" });
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Edit Intensity, 50" })),
    );
  });
});

