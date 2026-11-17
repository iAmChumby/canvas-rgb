import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { refreshAll } from "@/lib/store";
import { waitForPatternGuard } from "@/test/historyGuard";
import { Rail } from "@/shell/Rail";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { EffectsView } from "@/views/EffectsView";
import { PatternEditorPage } from "@/views/patterns/PatternEditorPage";

// The pattern editor's unsaved guard (#474): strokes that are not saved are never dropped
// without asking. Every way out asks, and the answer is the stack builder's own: Save keeps
// the work, Discard throws it away and leaves, Keep editing stays.
//
// The daemon is the demo backend, so the save in these tests is a real write to a real store.

const { push } = vi.hoisted(() => ({ push: vi.fn() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push, replace: vi.fn(), prefetch: vi.fn(), back: vi.fn() }),
  usePathname: () => "/patterns/",
}));

// Next's client-side link: the click handler runs first, and the link navigates only when it
// was not refused. The real Link needs the app router, which a unit test does not mount.
vi.mock("next/link", () => ({
  default: ({
    href,
    onClick,
    children,
  }: {
    href: string;
    onClick?: (e: { preventDefault: () => void; defaultPrevented: boolean }) => void;
    children?: React.ReactNode;
  }) => (
    <a
      href={href}
      onClick={(e) => {
        onClick?.(e);
        if (e.defaultPrevented) return;
        e.preventDefault();
        push(href);
      }}
    >
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
/**
 * The pattern the page opens: one built in memory, all off. It is not in the library, so what a
 * test saves cannot change what the next test opens.
 */
const PATTERN = "/patterns/?built=ring:12&state=patterns";
/** The name the built pattern is saved under. */
const PATTERN_ID = "pattern:Preview ring 12";

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
});

beforeEach(() => {
  // Every test opens the same pattern: one built in memory, all off. What a test saves cannot
  // change what the next test opens.
  window.history.replaceState({}, "", PATTERN);
});

afterEach(() => {
  cleanup();
  push.mockClear();
});

/** The editor, open on the seeded pattern. */
async function mount(): Promise<void> {
  await refreshAll();
  render(
    <>
      <Rail pathname="/patterns/" />
      <PatternEditorPage />
    </>,
  );
  await screen.findByRole("button", { name: "Fill all" });
}

/** Paints every LED, so the document differs from the one the page loaded. */
async function dirty(): Promise<void> {
  fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
  await waitFor(() => expect(screen.getByText("Unsaved changes")).toBeTruthy());
}

/** The prompt the guard opens. */
const prompt = () => screen.findByRole("alertdialog", { name: "Unsaved changes" });

const railLink = () => screen.getByRole("link", { name: /^Scenes/ });

describe("leaving the pattern editor", () => {
  it("asks before the Cancel key leaves unsaved strokes, and Discard goes on", async () => {
    await mount();
    await dirty();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await prompt()).toBeTruthy();
    // The page has not gone: the strokes are still the editor's.
    expect(push).not.toHaveBeenCalled();

    fireEvent.click(within(await prompt()).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/effects/"));
  });

  it("asks before a rail click leaves unsaved strokes, and Discard goes on", async () => {
    await mount();
    await dirty();

    fireEvent.click(railLink());
    expect(await prompt()).toBeTruthy();
    expect(push).not.toHaveBeenCalled();

    fireEvent.click(within(await prompt()).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/"));
  });

  it("asks before the browser's Back leaves unsaved strokes, and Discard goes back", async () => {
    // A page behind the pattern editor, so a Back has somewhere to land.
    window.history.replaceState({}, "", `/devices/?state=patterns`);
    window.history.pushState({}, "", PATTERN);
    await mount();
    await dirty();
    // The guard's entry and its listener land in the dirty effect after the change.
    await waitForPatternGuard();

    window.history.back();
    expect(await prompt()).toBeTruthy();
    // The page has not gone: the guard's own entry is on top of it.
    expect(window.location.pathname).toBe("/patterns/");

    fireEvent.click(within(await prompt()).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(window.location.pathname).toBe("/devices/"));
    expect(push).not.toHaveBeenCalled();
  });

  it("warns the browser only while the strokes are unsaved", async () => {
    await mount();
    const clean = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(clean);
    expect(clean.defaultPrevented).toBe(false);

    await dirty();
    const changed = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(changed);
    expect(changed.defaultPrevented).toBe(true);
  });

  it("keeps the strokes and the page when the answer is Keep editing", async () => {
    await mount();
    await dirty();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    const dialog = await prompt();
    fireEvent.click(within(dialog).getByRole("button", { name: "Keep editing" }));

    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
    expect(push).not.toHaveBeenCalled();
    // The strokes are still the editor's, and the mark still says so.
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
  });

  it("saves the pattern and leaves when the answer is Save", async () => {
    await mount();
    await dirty();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    const dialog = await prompt();
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(push).toHaveBeenCalledWith("/effects/"));
    // The pattern the daemon holds is the one that was painted.
    const saved = await api.pattern(PATTERN_ID);
    expect(saved.frames[0]?.colors.every((colour) => colour !== OFF)).toBe(true);
  });

  it("leaves at once when there is nothing to lose", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/effects/"));
    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
  });

  it("asks with the stack builder's own words", async () => {
    await mount();
    await dirty();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    const dialog = await prompt();
    expect(within(dialog).getByText("Save your changes?")).toBeTruthy();
    expect(
      within(dialog).getByText("The pattern has changes that are not saved. Leaving now loses them."),
    ).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Keep editing" })).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Discard" })).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeTruthy();
  });
});

describe("leaving the pattern panel in the effects view", () => {
  /** The effects page with the pattern panel open on the seeded pattern. */
  async function openPanel(): Promise<HTMLElement> {
    window.history.replaceState({}, "", "/effects/?state=patterns");
    await refreshAll();
    render(
      <>
        <Rail pathname="/effects/" />
        <StripSlotsProvider>
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
          <EffectsView />
        </StripSlotsProvider>
      </>,
    );
    // The inspector's own key opens the pattern's editor.
    fireEvent.click(await screen.findByRole("button", { name: "Edit the pattern Intake" }));
    return screen.findByLabelText(/, a pattern$/);
  }

  it("asks before a rail click leaves the panel's strokes", async () => {
    const editor = await openPanel();
    fireEvent.click(within(editor).getByRole("button", { name: "Fill all" }));
    await waitFor(() => expect(screen.getByText("Unsaved changes")).toBeTruthy());

    fireEvent.click(railLink());
    expect(await prompt()).toBeTruthy();
    expect(push).not.toHaveBeenCalled();
    // The panel is still open: the strokes are the editor's.
    expect(screen.getByLabelText(/, a pattern$/)).toBeTruthy();
  });

  it("saves the pattern and carries the rail link out when the answer is Save", async () => {
    const editor = await openPanel();
    fireEvent.click(within(editor).getByRole("button", { name: "Fill all" }));
    await waitFor(() => expect(screen.getByText("Unsaved changes")).toBeTruthy());

    fireEvent.click(railLink());
    const dialog = await prompt();
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    // The page change the rail asked for still happens, and the pattern is saved.
    await waitFor(() => expect(push).toHaveBeenCalledWith("/"));
    const saved = await api.pattern("pattern:Intake");
    expect(saved.frames[0]?.colors.every((colour) => colour !== OFF)).toBe(true);
  });
});
