import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { INITIAL_SETTINGS, LAYOUT_DESK } from "@/lib/fixtures";
import { patchSnap } from "@/lib/store";
import { LayoutView } from "@/views/LayoutView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The layout page keeps undo and redo in the status strip (R4), beside the page's name and the
// save state: they undo work done on the desk, and the save state says when there is some.

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/",
}));

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

afterEach(() => {
  cleanup();
});

describe("undo and redo in the strip", () => {
  it("undo takes a change back and redo takes it again, and the save state follows", async () => {
    patchSnap({
      layouts: [LAYOUT_DESK],
      activeLayout: LAYOUT_DESK.id,
      layoutsError: null,
      devices: [],
      effects: [],
      settings: INITIAL_SETTINGS,
    });
    render(
      <ToastProvider>
        <StripSlotsProvider>
          <header data-testid="strip">
            <StripSlot kind="title" />
            <StripSlot kind="actions" />
          </header>
          <LayoutView />
        </StripSlotsProvider>
      </ToastProvider>,
    );
    const strip = screen.getByTestId("strip");
    // The desk has its own pair of keys for a phone (hidden by css), so the strip's are asked for
    // inside the strip.
    const undo = await within(strip).findByRole("button", { name: "Undo the last change" });
    const redo = within(strip).getByRole("button", { name: "Redo the change that was undone" });
    expect(screen.getAllByRole("button", { name: "Undo the last change" })).toHaveLength(2);
    // Nothing was changed yet: there is nothing to undo or redo, and the layout is saved.
    expect((undo as HTMLButtonElement).disabled).toBe(true);
    expect((redo as HTMLButtonElement).disabled).toBe(true);
    expect(strip.textContent).toContain("Saved");

    // A change on the desk: the render area goes to the whole desk.
    const renderGroups = screen.getAllByRole("radiogroup", { name: "How much of the desk the effect is drawn at" });
    const all = renderGroups[0]!.querySelectorAll<HTMLButtonElement>("[role=radio]")[2]!;
    expect(LAYOUT_DESK.render_scale).toBe(50);
    fireEvent.click(all);
    await waitFor(() => expect(all.getAttribute("aria-checked")).toBe("true"));
    expect((undo as HTMLButtonElement).disabled).toBe(false);
    expect(strip.textContent).toContain("Unsaved");

    // Undo: the render area is back at a half, redo is open, and the layout is saved again.
    fireEvent.click(undo);
    await waitFor(() => expect(all.getAttribute("aria-checked")).toBe("false"));
    expect((undo as HTMLButtonElement).disabled).toBe(true);
    expect((redo as HTMLButtonElement).disabled).toBe(false);
    expect(strip.textContent).toContain("Saved");

    // Redo: the change is back.
    fireEvent.click(redo);
    await waitFor(() => expect(all.getAttribute("aria-checked")).toBe("true"));
    expect((redo as HTMLButtonElement).disabled).toBe(true);
    expect(strip.textContent).toContain("Unsaved");
  });
});
