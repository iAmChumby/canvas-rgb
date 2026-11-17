import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { api } from "@/lib/endpoints";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// New effect > Pattern (0.16.0) against the demo backend: the device step, the editor it opens,
// the name and the save, and the delete guard's refusal. `?state=patterns` seeds the two demo
// patterns and makes the Gaming scene's case fans play `pattern:Intake`, so a pattern is in the
// library, on the lights and in a scene at once.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=patterns");
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

const setState = (tokens: string) =>
  window.history.replaceState({}, "", `/effects/?state=${tokens}`);

async function mount() {
  await refreshAll();
  render(
    <>
      <NotifyHost />
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>
    </>,
  );
}

/** New effect, then Pattern: the device step. */
async function deviceStep() {
  await mount();
  fireEvent.click(await screen.findByRole("button", { name: "New effect" }));
  const menu = await screen.findByRole("dialog", { name: "New effect" });
  fireEvent.click(within(menu).getByRole("button", { name: "Pattern" }));
  return screen.findByTestId("build-on");
}

/** New effect > Pattern, then one device's row: the pattern editor on it. */
async function editorOn(deviceLabel = "Lian Li UNI FAN · Front fan") {
  const step = await deviceStep();
  fireEvent.click(within(step).getByRole("button", { name: `Build on ${deviceLabel}` }));
  return screen.findByLabelText(/, a pattern$/);
}

/** The name field of a new pattern, which the editor's head follows. */
const nameField = () => screen.getByLabelText("Name") as HTMLInputElement;

describe("a new pattern", () => {
  it("the device step groups lights of one kind into one entry", async () => {
    const step = await deviceStep();
    // The three fan-hub lights are all a ring of 40: one entry, not three.
    expect(within(step).getAllByText("ring of 40")).toHaveLength(1);
    // Two kinds have three lights each (the fan hubs and the DRAM strips), one row apiece.
    expect(within(step).getAllByText("3 devices")).toHaveLength(2);
    // The two connected DRAM sticks are one entry too, and the unplugged third is the same kind.
    expect(within(step).getAllByText("strip of 8")).toHaveLength(1);
    // No abstract shapes: every row is one of the owner's own devices, named.
    expect(within(step).getByRole("button", { name: "Build on Lian Li UNI FAN · Front fan" })).toBeTruthy();
    expect(within(step).getByRole("button", { name: "Build on Arctic Liquid Freezer III · Strip" })).toBeTruthy();
  });

  it("choosing a device opens the editor with an all-off pattern built on it", async () => {
    const editor = await editorOn();
    // The editor's head says what it was built on and that nothing is lit yet.
    expect(editor.textContent).toContain("ring of 40");
    expect(editor.textContent).toContain("of 40 on");
    expect(within(editor).getByText("0")).toBeTruthy();
    // The name is the next free "Pattern n", in a field and in the head above the device.
    expect(nameField().value).toMatch(/^Pattern \d+$/);
    expect(screen.getByRole("heading", { name: nameField().value })).toBeTruthy();
    // Nothing to lose: the Cancel key closes the panel at once.
    fireEvent.click(within(editor).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByLabelText(/, a pattern$/)).toBeNull();
  });

  it("the Cancel key asks before it drops unsaved strokes, and Discard closes the panel", async () => {
    const editor = await editorOn();
    fireEvent.click(within(editor).getByRole("button", { name: "Fill all" }));
    expect(await screen.findByText("Unsaved changes")).toBeTruthy();

    fireEvent.click(within(editor).getByRole("button", { name: "Cancel" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    // The panel is still open: the strokes are the editor's.
    expect(screen.getByLabelText(/, a pattern$/)).toBeTruthy();

    fireEvent.click(within(dialog).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(screen.queryByLabelText(/, a pattern$/)).toBeNull());
  });

  it("Keep editing keeps the strokes and the panel", async () => {
    const editor = await editorOn();
    fireEvent.click(within(editor).getByRole("button", { name: "Fill all" }));
    expect(await screen.findByText("Unsaved changes")).toBeTruthy();

    fireEvent.click(within(editor).getByRole("button", { name: "Cancel" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Keep editing" }));

    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
    // The panel is open and the strokes are still in it.
    expect(screen.getByLabelText(/, a pattern$/)).toBeTruthy();
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
  });

  it("Save puts the whole document and the tile appears", async () => {
    const editor = await editorOn();
    fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
    // The library shows it, drawn as its dots, and the inspector is on it.
    expect(await screen.findByRole("button", { name: "Edit the pattern Pattern 1" })).toBeTruthy();
    const saved = await api.pattern("Pattern 1");
    expect(saved.id).toBe("pattern:Pattern 1");
    expect(saved.built_on).toEqual({ led_count: 40, arrangement: "ring" });
    expect(saved.motion).toEqual({ type: "none" });
    expect(saved.frames).toHaveLength(1);
    expect(saved.frames[0]?.hold).toBe(0);
    expect(saved.frames[0]?.colors).toHaveLength(40);
    expect(saved.frames[0]?.colors.every((colour) => colour === "#000000")).toBe(true);
  });

  it("a taken name shows the 409 sentence", async () => {
    const editor = await editorOn();
    fireEvent.change(nameField(), { target: { value: "Intake" } });
    fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
    // The store's sentence, and the editor stays open with the work in it.
    expect(await screen.findByText(/is already another pattern's name/)).toBeTruthy();
    expect(screen.getByLabelText(/, a pattern$/)).toBeTruthy();
    // Nothing was written: the saved Intake is still the ring of 12 it was, not this one.
    const still = await api.pattern("Intake");
    expect(still.built_on.led_count).toBe(12);
    expect(still.frames[0]?.colors).toHaveLength(12);
    fireEvent.click(within(editor).getByRole("button", { name: "Cancel" }));
  });

  it("a kind with no connected light can be chosen and says so", async () => {
    setState("patterns,pattern-unplugged");
    const step = await deviceStep();
    // The three DRAM sticks are unplugged: the kind is still offered, and says why it is dark.
    expect(within(step).getByText("no device of this kind is connected")).toBeTruthy();
    fireEvent.click(within(step).getByRole("button", { name: "Build on ENE DRAM · DRAM" }));
    const editor = await screen.findByLabelText(/, a pattern$/);
    expect(editor.textContent).toContain("strip of 8");
    // The daemon's note comes back with the first draft, in the editor's own row.
    expect(await screen.findByText("no device of this kind is connected")).toBeTruthy();
    setState("patterns");
  });

  it("Delete shows the scenes the pattern is in and Delete anyway sends force", async () => {
    await mount();
    // The active scene plays pattern:Intake, so the inspector opens on it.
    const tile = await screen.findByRole("button", { name: "Edit the pattern Intake" });
    expect(tile).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: "Delete pattern" }));
    // The daemon refuses and names the scenes; nothing has gone yet.
    const refusal = await screen.findByText(/Intake plays in Gaming/);
    expect(refusal.textContent).toContain("Gaming");
    expect(screen.getByRole("button", { name: "Edit the pattern Intake" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Delete anyway" }));
    // Deleting anyway is the forced delete: the pattern and its tile are gone.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Edit the pattern Intake" })).toBeNull(),
    );
    const left = (await api.patterns()).patterns.map((pattern) => pattern.id);
    expect(left).not.toContain("pattern:Intake");
  });
});
