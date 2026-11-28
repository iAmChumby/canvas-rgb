import { act, cleanup, fireEvent, render, within } from "@testing-library/react";
import type { ComponentType } from "react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { patchSnap, refreshDevices, refreshDisplays, refreshEffects, refreshFans, refreshSettings } from "@/lib/store";
import { StripSlotsProvider } from "@/shell/PageHeader";
import { DevicesView } from "@/views/DevicesView";
import { DisplaysView } from "@/views/DisplaysView";
import { EffectsView } from "@/views/EffectsView";
import { FansView } from "@/views/FansView";
import { SettingsView } from "@/views/SettingsView";

// A poll that fails and recovers moves nothing (#576, K1). jsdom has no layout, so these tests
// hold what makes the offset stable: the page's stale row is there with and without the note, and
// the page is made of the same blocks in both states (the #109 pattern).

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

const NONE = {
  statusError: null,
  fansError: null,
  devicesError: null,
  displaysError: null,
  settingsError: null,
  effectsError: null,
};

afterEach(() => {
  cleanup();
  patchSnap(NONE);
});

interface Page {
  name: string;
  View: ComponentType;
  read: () => Promise<void>;
  area: Exclude<keyof typeof NONE, "statusError">;
  message: string;
  beat: string;
}

const PAGES: Page[] = [
  { name: "Fans", View: FansView, read: () => refreshFans(), area: "fansError", message: "The fan readings may be out of date.", beat: "Retrying every second." },
  { name: "Devices", View: DevicesView, read: () => refreshDevices(), area: "devicesError", message: "The device list may be out of date.", beat: "Retrying every 4 s." },
  { name: "Displays", View: DisplaysView, read: () => refreshDisplays(), area: "displaysError", message: "The displays may be out of date.", beat: "Retrying every 4 s." },
  { name: "Settings", View: SettingsView, read: () => refreshSettings(), area: "settingsError", message: "The settings may be out of date.", beat: "Retrying every 4 s." },
  { name: "Effects", View: EffectsView, read: () => refreshEffects(), area: "effectsError", message: "The effect list may be out of date.", beat: "Retrying every 4 s." },
];

/** The page after its first read, with whatever it asks for on mount answered. */
async function open({ View, read }: Page): Promise<void> {
  window.history.replaceState({}, "", "/?state=complete");
  await act(async () => {
    await read();
  });
  render(
    <StripSlotsProvider>
      <main className="content">
        <View />
      </main>
    </StripSlotsProvider>,
  );
  await act(async () => {
    await new Promise((done) => setTimeout(done, 50));
  });
}

const row = () => document.querySelector<HTMLElement>('.rd-page > [data-slot="stale-notice"]');
/** What the page is made of, block by block: a block that comes or goes moves what is under it. */
const blocks = () => [...(document.querySelector(".rd-page")?.children ?? [])].map((c) => `${c.tagName} ${c.className}`);

describe("the stale note's reserved row", () => {
  it.each(PAGES)("$name keeps its content where it is when a read fails and recovers", async (page) => {
    await open(page);
    const fresh = row();
    expect(fresh).not.toBeNull();
    expect(fresh!.textContent).toBe("");
    const before = blocks();

    act(() => patchSnap({ [page.area]: "could not read it" }));
    expect(row()).toBe(fresh);
    expect(blocks()).toEqual(before);
    expect(fresh!.textContent).toContain(`${page.message} ${page.beat}`);
    // The full anatomy is one tap away, inside the row, and opening it adds no block (K2).
    expect(fresh!.querySelector("[data-anatomy='error-state']")).toBeNull();
    fireEvent.click(within(fresh!).getByRole("button", { name: "Details" }));
    expect(fresh!.querySelector("[data-anatomy='error-state']")?.textContent).toContain("could not read it");
    expect(within(fresh!).getByRole("button", { name: "Try again" })).toBeTruthy();
    expect(blocks()).toEqual(before);

    act(() => patchSnap({ [page.area]: null }));
    expect(row()).toBe(fresh);
    expect(fresh!.textContent).toBe("");
    expect(blocks()).toEqual(before);
  });

  it("promises no beat while canvas-rgb itself is not answering", async () => {
    await open(PAGES[3]!);
    act(() => patchSnap({ settingsError: "no reply", statusError: "no reply" }));
    expect(row()!.textContent).toContain("Retrying when canvas-rgb answers.");
  });
});
