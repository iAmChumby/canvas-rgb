import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { waitForStackGuard } from "@/test/historyGuard";
import { trackHistory, type HistoryTracker } from "@/test/historyTracker";
import { Rail } from "@/shell/Rail";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { EffectsView } from "@/views/EffectsView";

// What the history looks like after each way out of the stack builder with unsaved changes, and
// where one Back lands (#484, the stack builder's half of #474). Every shape the builder can be
// opened in is crossed with every way out and every answer, and the whole list is asserted, the
// forward entries included: an entry left behind is a page the browser's Back or Forward reopens.

const { push, replace } = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push, replace, prefetch: vi.fn(), back: vi.fn() }),
  usePathname: () => "/effects/",
}));

// Next's client-side link: the click handler runs first, and the link navigates only when it was
// not refused.
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

/** The builder's own entry, as the page's `goStack` makes it. */
const STACK = "/effects/?stack=Sunset%20glow&state=complete,stack";
const EFFECTS = "/effects/?state=complete,stack";
const SCENES = "/?state=complete,stack";
/** The guard's entry, as the tracker prints it. */
const GUARD = " (guard)";

/** How the builder was reached: the entries behind it, or none on a direct load. */
const SHAPES: { name: string; behind: string[] }[] = [
  { name: "a direct load", behind: [] },
  { name: "opened from Effects", behind: [EFFECTS] },
  { name: "opened from Scenes", behind: [SCENES, EFFECTS] },
];

/** The model of the history a test asserts on, while it has one. */
let tracker: HistoryTracker | null = null;

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
  Range.prototype.getClientRects = (() =>
    ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList) as typeof Range.prototype.getClientRects;
  Range.prototype.getBoundingClientRect = (() =>
    ({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) }) as DOMRect) as typeof Range.prototype.getBoundingClientRect;
});

beforeEach(() => {
  // The router changes the real history, as Next's does: a push adds an entry, a replace
  // rewrites the current one.
  push.mockImplementation((href: string) => window.history.pushState({}, "", href));
  replace.mockImplementation((href: string) => window.history.replaceState({}, "", href));
});

afterEach(() => {
  cleanup();
  push.mockReset();
  replace.mockReset();
  tracker?.stop();
  tracker = null;
});

const here = () => window.location.pathname + window.location.search;

/** Puts the browser in the shape: the entries behind (if any), then the builder's own. */
function arrive(behind: string[]): HistoryTracker {
  window.history.replaceState({}, "", behind[0] ?? STACK);
  tracker = trackHistory();
  for (const url of behind.slice(1)) window.history.pushState({}, "", url);
  // The page opens the builder with its own push, which carries the route's state.
  if (behind.length > 0) window.history.pushState({ canvasRgbStack: "Sunset glow" }, "", STACK);
  return tracker;
}

/**
 * Waits until the whole history, the forward entries included, is exactly `urls`, with the document
 * on entry `at` (the last one unless said).
 */
async function settled(urls: string[], at = urls.length - 1): Promise<void> {
  await waitFor(() => {
    expect(tracker?.urls()).toEqual(urls);
    expect(tracker?.index).toBe(at);
    expect(here()).toBe(tracker?.entries[at]?.url);
  });
}

function page() {
  return (
    <>
      <NotifyHost />
      <Rail pathname="/effects/" />
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>
    </>
  );
}

/** The builder, open on the seeded stack with its top layer's eye turned: unsaved, guard up. */
async function dirtyBuilder(): Promise<void> {
  await refreshAll();
  render(page());
  await screen.findByTestId("stack-builder");
  await waitFor(() => expect(document.querySelectorAll(".layer-row").length).toBeGreaterThan(0));
  fireEvent.click(screen.getByRole("button", { name: /(Hide|Show) Sparkles/ }));
  await waitFor(() => expect(screen.getByText("Unsaved")).toBeTruthy());
  await waitForStackGuard();
}

const prompt = () => screen.findByRole("alertdialog", { name: "Unsaved changes" });
const builderOpen = () => screen.queryByTestId("stack-builder") !== null;

/** Presses one key of the unsaved prompt. */
async function answer(name: "Discard" | "Save" | "Keep editing"): Promise<void> {
  fireEvent.click(within(await prompt()).getByRole("button", { name }));
}

/** Goes Forward onto the entry after the document's, and waits for the builder to be shut on it. */
async function forwardShut(): Promise<void> {
  const target = (tracker?.index ?? 0) + 1;
  window.history.forward();
  await waitFor(() => expect(tracker?.index).toBe(target));
  // The entry is the effects page, with no builder: a Forward never reopens it.
  await waitFor(() => expect(here()).toBe("/effects/"));
  expect(builderOpen()).toBe(false);
}

describe.each(SHAPES)("the stack builder, $name", ({ behind }) => {
  const came = behind.length;
  /** Where one Back from where the person ended goes: the page they came from, or out of the app. */
  const cameFrom = behind[behind.length - 1];

  beforeEach(() => {
    arrive(behind);
  });

  async function ready(): Promise<void> {
    await dirtyBuilder();
    await settled([...behind, STACK, STACK + GUARD]);
  }

  /** One Back from the end of a leave: the page the person came from, or out of the document. */
  async function backOnce(): Promise<void> {
    window.history.back();
    if (cameFrom === undefined) expect(tracker?.left).toBe(true);
    else await waitFor(() => expect(here()).toBe(cameFrom));
  }

  it.each(["Discard", "Save"] as const)("the close key, then %s, replaces the builder with the effects page", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    await answer(choice);
    await waitFor(() => expect(builderOpen()).toBe(false));
    // The builder's own entry became the effects page. The guard's copy of the builder is still
    // ahead, so the whole list says so.
    await settled([...behind, "/effects/", STACK + GUARD], came);
    expect(tracker?.left).toBe(false);
    await backOnce();
  });

  it.each(["Discard", "Save"] as const)("the close key, then %s, leaves no entry ahead that reopens the builder", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    await answer(choice);
    await settled([...behind, "/effects/", STACK + GUARD], came);
    await forwardShut();
    // The guard's old copy became the effects page it stands for.
    expect(tracker?.urls()).toEqual([...behind, "/effects/", "/effects/"]);
  });

  it.each(["Discard", "Save"] as const)("a rail link, then %s, replaces the builder's entry with the page it goes to", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
    await answer(choice);
    // One Back is the page before the builder, not a fresh builder: the builder's entry is gone, and
    // the guard's copy is ahead of the new page, inert.
    await settled([...behind, "/devices/", STACK + GUARD], came);
    expect(tracker?.left).toBe(false);
    await backOnce();
  });

  it.each(["Discard", "Save"] as const)("a rail link, then %s, leaves no entry ahead that reopens the builder", async (choice) => {
    await ready();
    fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
    await answer(choice);
    await settled([...behind, "/devices/", STACK + GUARD], came);
    // Forward onto the guard's copy, as a page that mounts for it: a fresh effects page opens no builder.
    window.history.forward();
    await waitFor(() => expect(here()).toBe(STACK));
    cleanup();
    render(page());
    await waitFor(() => expect(here()).toBe("/effects/"));
    expect(builderOpen()).toBe(false);
  });

  it.each(["Discard", "Save"] as const)("Back, then %s, goes to the page it came from and never leaves the document", async (choice) => {
    await ready();
    window.history.back();
    await prompt();
    await answer(choice);
    if (cameFrom === undefined) {
      // Nothing of the app is behind the builder: it closes onto the effects page, with the guard's
      // old copy ahead of it.
      await waitFor(() => expect(builderOpen()).toBe(false));
      await settled(["/effects/", STACK + GUARD], 0);
    } else {
      await waitFor(() => expect(here()).toBe(cameFrom));
      // Both entries ahead are spent: the builder's own was marked on the way past, the guard's copy
      // is the guard's. Neither opens a builder.
      await waitFor(() => expect(tracker?.urls()).toEqual([...behind, STACK + GUARD, STACK + GUARD]));
      expect(tracker?.index).toBe(came - 1);
      await waitFor(() => expect(builderOpen()).toBe(false));
    }
    expect(tracker?.left).toBe(false);
  });

  it.each(["Discard", "Save"] as const)("Back, then %s, leaves no entry ahead that reopens the builder", async (choice) => {
    await ready();
    window.history.back();
    await prompt();
    await answer(choice);
    if (cameFrom === undefined) {
      await settled(["/effects/", STACK + GUARD], 0);
      await forwardShut();
      return;
    }
    await waitFor(() => expect(here()).toBe(cameFrom));
    await waitFor(() => expect(tracker?.urls()).toEqual([...behind, STACK + GUARD, STACK + GUARD]));
    // Every Forward from here: each entry opens no builder.
    await forwardShut();
    await forwardShut();
    expect(tracker?.urls()).toEqual([...behind, "/effects/", "/effects/"]);
  });

  it.each(["the close key", "a rail link", "Back"])("%s, then Keep editing, keeps the builder and its guard", async (way) => {
    await ready();
    if (way === "the close key") fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    else if (way === "a rail link") fireEvent.click(screen.getByRole("link", { name: /^Devices/ }));
    else window.history.back();
    await answer("Keep editing");
    await settled([...behind, STACK, STACK + GUARD]);
    expect(builderOpen()).toBe(true);
    expect(screen.getByText("Unsaved")).toBeTruthy();
    // The guard is still up: the next Back asks again.
    window.history.back();
    expect(await prompt()).toBeTruthy();
    expect(tracker?.left).toBe(false);
  });

  it("the Save key keeps the builder and takes the guard's entry back", async () => {
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByText("Unsaved")).toBeNull());
    await settled([...behind, STACK, STACK + GUARD], came);
    expect(builderOpen()).toBe(true);
    await backOnce();
  });
});
