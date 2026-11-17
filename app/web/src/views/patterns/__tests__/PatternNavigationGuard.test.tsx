import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import type { Pattern } from "@/proto/Pattern";
import { refreshAll } from "@/lib/store";
import { waitForPatternGuard } from "@/test/historyGuard";
import { trackHistory, type HistoryTracker } from "@/test/historyTracker";
import { Rail } from "@/shell/Rail";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { EffectsView } from "@/views/EffectsView";
import { PatternEditorPage } from "@/views/patterns/PatternEditorPage";

// The pattern editor's unsaved guard (#474): strokes that are not saved are never dropped
// without asking. Every way out asks, and the answer is the stack builder's own: Save keeps
// the work, Discard throws it away and leaves, Keep editing stays.
//
// The daemon is the demo backend, so the save in these tests is a real write to a real store.

const { push, replace } = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push, replace, prefetch: vi.fn(), back: vi.fn() }),
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

/** The model of the history a test asserts on, while it has one. */
let tracker: HistoryTracker | null = null;

/** The seeded pattern the panel opens, as it was before any test saved over it. */
let intake: Pattern | null = null;

beforeAll(async () => {
  window.history.replaceState({}, "", "/effects/?state=patterns");
  intake = await api.pattern("pattern:Intake");
});

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
  // The router changes the real history, as Next's does: a push adds an entry, a replace
  // rewrites the current one.
  push.mockImplementation((href: string) => window.history.pushState({}, "", href));
  replace.mockImplementation((href: string) => window.history.replaceState({}, "", href));
  // Every test opens the same pattern: one built in memory, all off. What a test saves cannot
  // change what the next test opens.
  window.history.replaceState({}, "", PATTERN);
});

afterEach(() => {
  cleanup();
  push.mockClear();
  replace.mockClear();
  tracker?.stop();
  tracker = null;
  delete (window as { navigation?: unknown }).navigation;
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
    await waitFor(() => expect(replace).toHaveBeenCalledWith("/effects/"));
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

    await waitFor(() => expect(replace).toHaveBeenCalledWith("/effects/"));
    // The pattern the daemon holds is the one that was painted.
    const saved = await api.pattern(PATTERN_ID);
    expect(saved.frames[0]?.colors.every((colour) => colour !== OFF)).toBe(true);
  });

  it("leaves at once when there is nothing to lose", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(replace).toHaveBeenCalledWith("/effects/"));
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

// What the history looks like after each way out, and where one Back lands (#474). Every shape
// the editor can be opened in is crossed with every way out and every answer, and the whole
// history list is asserted, not only the page the person ends on: an entry left behind is a page
// the browser's Back reopens.

/** How the editor was reached: the entry behind it, or none on a direct load. */
const SHAPES: { name: string; behind: string | null }[] = [
  { name: "a direct load", behind: null },
  { name: "opened from Effects", behind: "/effects/?state=patterns" },
  { name: "opened from Scenes", behind: "/?state=patterns" },
];
/** The guard's entry, as the tracker prints it. */
const GUARD = " (guard)";
const here = () => window.location.pathname + window.location.search;

/**
 * Puts the browser in the shape: the entry behind (if any), then the editor's own. The Navigation
 * API's answer to "is there an entry of this app behind" is the one a browser gives for it.
 */
function arrive(behind: string | null, url: string): HistoryTracker {
  if (behind !== null) window.history.replaceState({}, "", behind);
  else window.history.replaceState({}, "", url);
  tracker = trackHistory();
  if (behind !== null) window.history.pushState({}, "", url);
  Object.defineProperty(window, "navigation", { configurable: true, value: { canGoBack: behind !== null } });
  return tracker;
}

/**
 * Waits until the history up to the document's entry is exactly `urls`, with the document on its
 * last one. Entries after it are the browser's forward list: a Back that was taken leaves them,
 * and no Back reaches them.
 */
async function settled(urls: string[], at = urls.length - 1): Promise<void> {
  await waitFor(() => {
    expect(tracker?.urls().slice(0, at + 1)).toEqual(urls);
    expect(tracker?.index).toBe(at);
    expect(here()).toBe(tracker?.entries[at]?.url);
  });
}

/** Presses one key of the unsaved prompt. */
async function answer(name: "Discard" | "Save" | "Keep editing"): Promise<void> {
  fireEvent.click(within(await prompt()).getByRole("button", { name }));
}

describe.each(SHAPES)("the pattern page, $name", ({ behind }) => {
  const before = behind === null ? [] : [behind];
  const editor = PATTERN;

  beforeEach(() => {
    arrive(behind, PATTERN);
  });

  /** Opens the editor, makes it dirty and waits for the guard's entry. */
  async function ready(): Promise<void> {
    await mount();
    await dirty();
    await waitForPatternGuard();
    await settled([...before, editor, editor + GUARD]);
  }

  /** One Back from where the person ended: where they came from, or out of the app on a direct load. */
  async function backOnce(): Promise<void> {
    window.history.back();
    if (behind === null) expect(tracker?.left).toBe(true);
    else await waitFor(() => expect(here()).toBe(behind));
  }

  it.each(["Discard", "Save"] as const)("Cancel, then %s, leaves the effects page and no guard entry", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await answer(choice);
    // The editor's own entry became the effects page: nothing of the editor is left to reopen.
    await settled([...before, "/effects/"]);
    expect(tracker?.left).toBe(false);
    await backOnce();
  });

  it.each(["Discard", "Save"] as const)("a rail link, then %s, leaves no guard entry behind the page it goes to", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
    await answer(choice);
    await settled([...before, editor, "/devices/"]);
    // One Back is the editor's page, a fresh one: the guard's copy of it is gone.
    window.history.back();
    await waitFor(() => expect(here()).toBe(editor));
    expect(tracker?.urls()).toEqual([...before, editor, "/devices/"]);
  });

  it.each(["Discard", "Save"] as const)("Back, then %s, goes to the page it came from and never leaves the document", async (choice) => {
    await ready();
    window.history.back();
    await prompt();
    await answer(choice);
    if (behind === null) {
      // Nothing of the app is behind the editor: it closes onto the effects page, one entry.
      await settled(["/effects/"]);
    } else {
      await waitFor(() => expect(here()).toBe(behind));
      expect(tracker?.urls()).toEqual([behind, editor, editor + GUARD]);
      expect(tracker?.index).toBe(0);
    }
    expect(tracker?.left).toBe(false);
  });

  it.each(["Cancel", "a rail link", "Back"])("%s, then Keep editing, keeps the editor and its guard", async (way) => {
    await ready();
    if (way === "Cancel") fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    else if (way === "a rail link") fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
    else window.history.back();
    await answer("Keep editing");
    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
    await settled([...before, editor, editor + GUARD]);
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
    // The guard is still up: the next Back asks again.
    window.history.back();
    expect(await prompt()).toBeTruthy();
    expect(tracker?.left).toBe(false);
  });

  it("the Save key keeps the page and takes the guard's entry back", async () => {
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await settled([...before, editor]);
    await waitFor(() => expect(screen.getByText("No unsaved changes")).toBeTruthy());
    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
    await backOnce();
  });
});

describe.each(SHAPES.filter((shape) => shape.name !== "opened from Effects"))(
  "the pattern panel in the effects view, $name",
  ({ behind }) => {
    const before = behind === null ? [] : [behind];
    const page = "/effects/?state=patterns";

    beforeEach(async () => {
      // A Save in an earlier test painted Intake: put it back, so Fill all changes it again.
      if (intake) await api.putPattern("pattern:Intake", intake);
      arrive(behind, page);
    });

    /** The effects page with the panel open on the seeded pattern and its strokes unsaved. */
    async function ready(): Promise<HTMLElement> {
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
      fireEvent.click(await screen.findByRole("button", { name: "Edit the pattern Intake" }));
      const editor = await screen.findByLabelText(/, a pattern$/);
      fireEvent.click(within(editor).getByRole("button", { name: "Fill all" }));
      await waitFor(() => expect(screen.getByText("Unsaved changes")).toBeTruthy());
      await waitForPatternGuard();
      await settled([...before, page, page + GUARD]);
      return editor;
    }

    const panelOpen = () => screen.queryByLabelText(/, a pattern$/) !== null;

    async function backOnce(): Promise<void> {
      window.history.back();
      if (behind === null) expect(tracker?.left).toBe(true);
      else await waitFor(() => expect(here()).toBe(behind));
    }

    it.each(["Discard", "Save"] as const)("Cancel, then %s, closes the panel and leaves no guard entry", async (choice) => {
      const editor = await ready();
      fireEvent.click(within(editor).getByRole("button", { name: "Cancel" }));
      await answer(choice);
      await waitFor(() => expect(panelOpen()).toBe(false));
      await settled([...before, page]);
      expect(tracker?.left).toBe(false);
      await backOnce();
    });

    it.each(["Discard", "Save"] as const)("a rail link, then %s, leaves no guard entry behind the page it goes to", async (choice) => {
      await ready();
      fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
      await answer(choice);
      await settled([...before, page, "/devices/"]);
      window.history.back();
      await waitFor(() => expect(here()).toBe(page));
    });

    it.each(["Discard", "Save"] as const)("Back, then %s, goes to the page it came from and never leaves the document", async (choice) => {
      await ready();
      window.history.back();
      await prompt();
      await answer(choice);
      if (behind === null) {
        // The effects page is the first page: the panel closes on it, one entry.
        await waitFor(() => expect(panelOpen()).toBe(false));
        await settled([page]);
      } else {
        await waitFor(() => expect(here()).toBe(behind));
        expect(tracker?.urls()).toEqual([behind, page, page + GUARD]);
        expect(tracker?.index).toBe(0);
      }
      expect(tracker?.left).toBe(false);
    });

    it("the Save key closes the panel and leaves no guard entry", async () => {
      const editor = await ready();
      fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(panelOpen()).toBe(false));
      await settled([...before, page]);
      expect(tracker?.left).toBe(false);
      await backOnce();
    });

    it.each(["Cancel", "a rail link", "Back"])("%s, then Keep editing, keeps the panel and its guard", async (way) => {
      const editor = await ready();
      if (way === "Cancel") fireEvent.click(within(editor).getByRole("button", { name: "Cancel" }));
      else if (way === "a rail link") fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
      else window.history.back();
      await answer("Keep editing");
      await settled([...before, page, page + GUARD]);
      expect(panelOpen()).toBe(true);
      expect(screen.getByText("Unsaved changes")).toBeTruthy();
    });
  },
);
