import { useInsertionEffect } from "react";
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
    // The browser says there is an entry of the app behind the editor.
    Object.defineProperty(window, "navigation", { configurable: true, value: { canGoBack: true } });
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

  it("goes to the effects page on Back, Discard when the browser cannot say a page of the app is behind", async () => {
    // No Navigation API: `history.length` counts entries the app did not make, so the editor does
    // not guess it was opened from the app, and a Back that leaves closes onto the effects page.
    window.history.replaceState({}, "", `/devices/?state=patterns`);
    window.history.pushState({}, "", PATTERN);
    await mount();
    await dirty();
    await waitForPatternGuard();

    window.history.back();
    fireEvent.click(within(await prompt()).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(replace).toHaveBeenCalledWith("/effects/"));
    expect(window.location.pathname).not.toBe("/devices/");
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

describe("opening the pattern page by a client-side push", () => {
  /**
   * Next changes the history in the commit of the page it navigates to, in an insertion effect, so
   * while the new page renders `history.state` is still the entry being left.
   */
  function NextCommit({ to }: { to: string }) {
    useInsertionEffect(() => {
      window.history.pushState({}, "", to);
    }, [to]);
    return null;
  }

  it("opens the editor when the entry being left is a dirty panel's guard entry (#484)", async () => {
    // The entry being left: the effects page, with the panel's guard state on it.
    window.history.replaceState({ canvasRgbPatternGuard: true }, "", "/effects/?state=patterns");
    await refreshAll();
    render(
      <>
        <NextCommit to={PATTERN} />
        <PatternEditorPage />
      </>,
    );
    // The editor opens on the pattern, and nothing sent the page to the effects page.
    expect(await screen.findByRole("button", { name: "Fill all" })).toBeTruthy();
    expect(replace).not.toHaveBeenCalled();
  });

  it("a push that goes past a dirty panel's guard is not undone by the panel closing (#484)", async () => {
    if (intake) await api.putPattern("pattern:Intake", intake);
    arrive(null, "/effects/?state=patterns");
    await refreshAll();
    render(
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Edit the pattern Intake" }));
    fireEvent.click(within(await screen.findByLabelText(/, a pattern$/)).getByRole("button", { name: "Fill all" }));
    await waitForPatternGuard();
    await settled(["/effects/?state=patterns", "/effects/?state=patterns" + GUARD]);
    // A link that does not ask pushes the new page's entry on top of the guard's, and the panel goes.
    window.history.pushState({}, "", PATTERN);
    cleanup();
    await new Promise((resolve) => setTimeout(resolve, 50));
    await settled(["/effects/?state=patterns", "/effects/?state=patterns" + GUARD, PATTERN]);
  });

  it("still opens no editor on an entry that is the guard's own", async () => {
    window.history.replaceState({ canvasRgbPatternGuard: true }, "", PATTERN);
    render(<PatternEditorPage />);
    await expectInert();
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

describe("leaving the pattern panel on the narrow sheet", () => {
  const wide = () => window.matchMedia;
  let desktop: typeof window.matchMedia;

  beforeEach(async () => {
    // A Save in an earlier test painted Intake: put it back, so Fill all changes it again.
    if (intake) await api.putPattern("pattern:Intake", intake);
    desktop = wide();
    // 390 px: the sheet pushed over the gallery, with its own Back key.
    window.matchMedia = ((query: string) => ({
      matches: query.includes("max-width: 720px"),
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
    })) as unknown as typeof window.matchMedia;
    Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  });

  afterEach(() => {
    window.matchMedia = desktop;
  });

  /** The sheet's Back key, with the panel open on Intake and every LED filled. */
  async function dirtySheet(): Promise<void> {
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
    fireEvent.click(await screen.findByRole("button", { name: "Edit the pattern Intake" }));
    const editor = await screen.findByLabelText(/, a pattern$/);
    fireEvent.click(within(editor).getByRole("button", { name: "Fill all" }));
    await waitFor(() => expect(screen.getByText("Unsaved changes")).toBeTruthy());
    await waitForPatternGuard();
  }

  it("asks before the sheet's Back key drops the panel's strokes, and Keep editing keeps them", async () => {
    await dirtySheet();
    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    const dialog = await prompt();
    // The strokes are still the editor's while the prompt is up.
    expect(screen.getByLabelText(/, a pattern$/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Keep editing" }));
    expect(screen.getByLabelText(/, a pattern$/)).toBeTruthy();
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
  });

  it("closes the panel when the answer to the sheet's Back key is Discard", async () => {
    await dirtySheet();
    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    fireEvent.click(within(await prompt()).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(screen.queryByLabelText(/, a pattern$/)).toBeNull());
  });

  it("goes at once when the panel has nothing to lose", async () => {
    window.history.replaceState({}, "", "/effects/?state=patterns");
    await refreshAll();
    render(
      <>
        <StripSlotsProvider>
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
          <EffectsView />
        </StripSlotsProvider>
      </>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Edit the pattern Intake" }));
    await screen.findByLabelText(/, a pattern$/);
    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    await waitFor(() => expect(screen.queryByLabelText(/, a pattern$/)).toBeNull());
    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
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
 * Waits until the whole history, the forward entries included, is exactly `urls`, with the document
 * on entry `at` (the last one unless said). A forward entry the editor leaves behind is a page the
 * browser's Forward reopens, so it is part of what a test asserts.
 */
async function settled(urls: string[], at = urls.length - 1): Promise<void> {
  await waitFor(() => {
    expect(tracker?.urls()).toEqual(urls);
    expect(tracker?.index).toBe(at);
    expect(here()).toBe(tracker?.entries[at]?.url);
  });
}

/**
 * Goes Forward onto the entry after the document's and returns the page that mounts for it: a
 * fresh `PatternEditorPage`, as the router mounts one for an address it was not on. The editor of
 * the old page is gone first, so what is on screen is what the entry opens.
 */
async function forwardOnto(url: string): Promise<void> {
  window.history.forward();
  await waitFor(() => expect(here()).toBe(url));
  cleanup();
  render(<PatternEditorPage />);
}

/** The entry the Forward opened is inert: it sends the page to the effects page and opens no editor. */
async function expectInert(): Promise<void> {
  await waitFor(() => expect(replace).toHaveBeenCalledWith("/effects/"));
  expect(screen.queryByRole("button", { name: "Fill all" })).toBeNull();
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
    // The editor's own entry became the effects page. The guard's copy of the editor is still
    // ahead, so the whole list says so: it is the entry the next test walks onto.
    await settled([...before, "/effects/", editor + GUARD], before.length);
    expect(tracker?.left).toBe(false);
    await backOnce();
  });

  it.each(["Discard", "Save"] as const)("Cancel, then %s, leaves no entry ahead that reopens the editor", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await answer(choice);
    await settled([...before, "/effects/", editor + GUARD], before.length);
    // Forward onto the guard's old copy of the editor: it opens no editor and lands on the effects page.
    await forwardOnto(editor);
    await expectInert();
  });

  it.each(["Discard", "Save"] as const)("a rail link, then %s, replaces the editor's entry with the page it goes to", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
    await answer(choice);
    // The editor's own entry became the page the link goes to, so one Back is the page before the
    // editor and never a fresh editor. The guard's copy is ahead of the new page, inert (#484).
    await settled([...before, "/devices/", editor + GUARD], before.length);
    expect(tracker?.left).toBe(false);
    await backOnce();
  });

  it.each(["Discard", "Save"] as const)("a rail link, then %s, leaves no entry ahead that opens an editor", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
    await answer(choice);
    await settled([...before, "/devices/", editor + GUARD], before.length);
    // Forward onto the guard's copy: it opens no editor and lands on the effects page.
    await forwardOnto(editor);
    await expectInert();
  });

  it.each(["Discard", "Save"] as const)("Back, then %s, goes to the page it came from and never leaves the document", async (choice) => {
    await ready();
    window.history.back();
    await prompt();
    await answer(choice);
    if (behind === null) {
      // Nothing of the app is behind the editor: it closes onto the effects page, with the guard's
      // old copy ahead of it.
      await settled(["/effects/", editor + GUARD], 0);
    } else {
      await waitFor(() => expect(here()).toBe(behind));
      // Both entries ahead are spent: the editor's own was marked on the way past, the guard's copy
      // is the guard's. Neither opens an editor.
      await waitFor(() => expect(tracker?.urls()).toEqual([behind, editor + GUARD, editor + GUARD]));
      expect(tracker?.index).toBe(0);
    }
    expect(tracker?.left).toBe(false);
  });

  it.each(["Discard", "Save"] as const)("Back, then %s, leaves no entry ahead that reopens the editor", async (choice) => {
    await ready();
    window.history.back();
    await prompt();
    await answer(choice);
    if (behind === null) {
      await settled(["/effects/", editor + GUARD], 0);
    } else {
      await waitFor(() => expect(tracker?.urls()).toEqual([behind, editor + GUARD, editor + GUARD]));
      await waitFor(() => expect(here()).toBe(behind));
      // Every Forward from here: each entry opens no editor.
      await forwardOnto(editor);
      await expectInert();
      replace.mockClear();
      cleanup();
      window.history.forward();
      await waitFor(() => expect(here()).toBe(editor));
      render(<PatternEditorPage />);
      await expectInert();
    }
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
    await settled([...before, editor, editor + GUARD], before.length);
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
      await settled([...before, page, page + GUARD], before.length);
      expect(tracker?.left).toBe(false);
      await backOnce();
    });

    it.each(["Discard", "Save"] as const)("Cancel, then %s, leaves no entry ahead that reopens the panel", async (choice) => {
      const editor = await ready();
      fireEvent.click(within(editor).getByRole("button", { name: "Cancel" }));
      await answer(choice);
      await waitFor(() => expect(panelOpen()).toBe(false));
      await settled([...before, page, page + GUARD], before.length);
      // Forward onto the guard's old copy: the same effects page, with the panel shut.
      window.history.forward();
      await waitFor(() => expect(here()).toBe(page));
      expect(panelOpen()).toBe(false);
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
        await settled([page, page + GUARD], 0);
      } else {
        await waitFor(() => expect(here()).toBe(behind));
        await waitFor(() => expect(tracker?.urls()).toEqual([behind, page + GUARD, page + GUARD]));
        expect(tracker?.index).toBe(0);
      }
      expect(tracker?.left).toBe(false);
    });

    it("the Save key closes the panel and leaves no guard entry", async () => {
      const editor = await ready();
      fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(panelOpen()).toBe(false));
      await settled([...before, page, page + GUARD], before.length);
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
