import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { dismissNotifications, NotifyHost } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The stack builder's refusals (#168). They are said next to the control that refused, not as a
// pop-up: the message stands with the key that will not work.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=complete,stack,html");
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

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  // A key on the stage that is refused also says so as a note (#257); the note outlives the
  // page it came from, and the next test would find its words twice.
  act(() => dismissNotifications());
});

async function openBuilder(stack: string) {
  window.history.replaceState({}, "", `/effects/?stack=${encodeURIComponent(stack)}&state=complete,stack,html`);
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

/** The names of the layers, top first. */
function layerNames(): string[] {
  return screen
    .getAllByRole("listitem")
    .map((row) => row.querySelector(".layer-name")?.textContent ?? "")
    .filter(Boolean);
}

/** Picks a layer by its name and waits for its panel. */
async function pickLayer(name: string) {
  const row = await waitFor(() => {
    const hit = screen
      .getAllByRole("listitem")
      .find((item) => item.querySelector(".layer-name")?.textContent === name);
    if (!hit) throw new Error(`no layer row named ${name}`);
    return hit;
  });
  fireEvent.click(row);
  await screen.findByRole("button", { name: `Duplicate ${name}` });
}

/** Adds a layer from the library by the effect's name. */
async function addLayer(effect: string) {
  fireEvent.click(await screen.findByRole("button", { name: `Add ${effect}` }));
}

/** The layer's own panel: the picked layer's fields. */
const layerPanel = () => screen.getByRole("button", { name: /Duplicate / }).closest(".panel") as HTMLElement;

describe("the stack builder says a refusal where the control refused", () => {
  it("says a bad layer name under the name field", async () => {
    await openBuilder("Sunset glow");
    await waitFor(() => expect(layerNames()).toEqual(["Sparkles", "Heat", "Sunset"]));

    fireEvent.doubleClick(screen.getByText("Sunset"));
    const field = await screen.findByLabelText("Rename Sunset");
    fireEvent.change(field, { target: { value: "Sunset!" } });
    fireEvent.keyDown(field, { key: "Enter" });

    // The message stands with the field, and the field says it is wrong.
    const problem = await screen.findByText(/may only hold letters/);
    expect(problem.getAttribute("role")).toBe("alert");
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(field.getAttribute("aria-describedby")).toBe(problem.id);
  });

  it("says a preset cannot be saved before the stack is, under the preset keys", async () => {
    await openBuilder("new");
    await screen.findByRole("button", { name: "Save the current values as a preset" });

    fireEvent.click(screen.getByRole("button", { name: "Save the current values as a preset" }));
    const problem = await screen.findByText("Save the stack before saving a preset");
    expect(problem.getAttribute("role")).toBe("alert");
  });

  it("says the stack is full beside the layer it would not duplicate", async () => {
    await openBuilder("Eight layers");
    await pickLayer("Base");

    fireEvent.click(screen.getByRole("button", { name: "Duplicate Base" }));
    // The refusal is also a note, so the words are on the page twice; the panel's is asked about.
    const problem = await within(layerPanel()).findByText("A stack holds 8 layers");
    expect(problem.getAttribute("role")).toBe("alert");
    // The message is in the picked layer's own panel.
    expect(layerPanel().contains(problem)).toBe(true);
  });

  it("says the HTML layers are full beside the layer it would not duplicate", async () => {
    await openBuilder("Sunset glow");
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    await addLayer("Sample HTML");
    await waitFor(() => expect(layerNames()).toHaveLength(4));
    await addLayer("Ink Drop");
    await waitFor(() => expect(layerNames()).toHaveLength(5));

    await pickLayer("Sample HTML");
    fireEvent.click(screen.getByRole("button", { name: "Duplicate Sample HTML" }));
    const problem = await within(layerPanel()).findByText("A stack holds 2 HTML layers");
    expect(problem.getAttribute("role")).toBe("alert");
    expect(layerPanel().contains(problem)).toBe(true);
  });

  it("says the HTML layers are full beside the effect it would not take", async () => {
    await openBuilder("Sunset glow");
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    await addLayer("Sample HTML");
    await waitFor(() => expect(layerNames()).toHaveLength(4));
    await addLayer("Ink Drop");
    await waitFor(() => expect(layerNames()).toHaveLength(5));

    // The picked layer is not an HTML one, and the HTML layers are full.
    await pickLayer("Sunset");
    fireEvent.click(screen.getByRole("combobox", { name: "Effect" }));
    const option = await screen.findByRole("option", { name: /Wet Smear/ });
    fireEvent.click(option);

    const problem = await within(layerPanel()).findByText("A stack holds 2 HTML layers");
    expect(problem.getAttribute("role")).toBe("alert");
    expect(layerPanel().contains(problem)).toBe(true);
  });

  it("says why an HTML layer cannot be added, on the tile that cannot add it", async () => {
    await openBuilder("Sunset glow");
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    await addLayer("Sample HTML");
    await waitFor(() => expect(layerNames()).toHaveLength(4));
    await addLayer("Ink Drop");
    await waitFor(() => expect(layerNames()).toHaveLength(5));

    const tile = screen.getByRole("button", { name: "Add Wet Smear" });
    // The reason stands on the tile, and the tile will not add it.
    expect(within(tile).getByText("A stack holds 2 HTML layers")).toBeTruthy();
    expect(tile).toHaveProperty("disabled", true);
  });

  it("says the HTML layers are full when the cap is reached while an effect is read", async () => {
    await openBuilder("Sunset glow");
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    await addLayer("Sample HTML");
    await waitFor(() => expect(layerNames()).toHaveLength(4));

    // The effect the layer is being given is read slowly: the cap is reached while it waits.
    const original = api.effect.bind(api);
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let blocked = false;
    vi.spyOn(api, "effect").mockImplementation(async (id: string) => {
      if (id === "html:ink-drop" && !blocked) {
        blocked = true;
        await gate;
      }
      return original(id);
    });

    await pickLayer("Sunset");
    fireEvent.click(screen.getByRole("combobox", { name: "Effect" }));
    fireEvent.click(within(await screen.findByRole("listbox", { name: "Effect" })).getByRole("option", { name: /^Ink Drop/ }));
    await waitFor(() => expect(blocked).toBe(true));

    // While the effect is read, another HTML layer lands: the stack is full of them.
    await addLayer("Wet Smear");
    await waitFor(() => expect(layerNames()).toHaveLength(5));

    await act(async () => {
      release();
    });
    const problem = await within(layerPanel()).findByText("A stack holds 2 HTML layers");
    expect(problem.getAttribute("role")).toBe("alert");
    expect(layerPanel().contains(problem)).toBe(true);
  });

  it("says the HTML layers are full when the cap is reached while a layer is added", async () => {
    await openBuilder("Sunset glow");
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    await addLayer("Sample HTML");
    await waitFor(() => expect(layerNames()).toHaveLength(4));

    // The layer being added is read slowly: the cap is reached while it waits.
    const original = api.effect.bind(api);
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let blocked = false;
    vi.spyOn(api, "effect").mockImplementation(async (id: string) => {
      if (id === "html:wet-smear" && !blocked) {
        blocked = true;
        await gate;
      }
      return original(id);
    });

    fireEvent.click(await screen.findByRole("button", { name: "Add Wet Smear" }));
    await waitFor(() => expect(blocked).toBe(true));

    // While the layer is read, another layer becomes an HTML one: the stack is full of them.
    await pickLayer("Sunset");
    fireEvent.click(screen.getByRole("combobox", { name: "Effect" }));
    fireEvent.click(within(await screen.findByRole("listbox", { name: "Effect" })).getByRole("option", { name: /^Ink Drop/ }));
    await waitFor(() => expect(layerNames()).toHaveLength(4));

    await act(async () => {
      release();
    });
    const problem = await screen.findByRole("alert");
    expect(problem.textContent).toBe("A stack holds 2 HTML layers");
    // The message stands under the library's tiles.
    expect(document.querySelector(".rd-library-panel")?.contains(problem)).toBe(true);
    // The layer was not added.
    expect(layerNames()).toHaveLength(4);
  });
});
