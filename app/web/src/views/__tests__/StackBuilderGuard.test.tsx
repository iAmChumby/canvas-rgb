import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { refreshAll } from "@/lib/store";
import { waitForStackGuard } from "@/test/historyGuard";
import { Rail } from "@/shell/Rail";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { EffectsView } from "@/views/EffectsView";

// The builder's unsaved guard (D95.5.8) on the ways out it does not own: the browser's Back
// and an in-app page change. Every one asks, and every one lands exactly one page away.

const { push } = vi.hoisted(() => ({ push: vi.fn() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push, replace: vi.fn(), prefetch: vi.fn(), back: vi.fn() }),
  usePathname: () => "/effects/",
}));

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

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
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
  push.mockClear();
});

async function mount() {
  await refreshAll();
  return render(
    <>
      <NotifyHost />
      <Rail pathname="/effects/" />
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>
    </>,
  );
}

async function builderReady() {
  await screen.findByTestId("stack-builder");
  await waitFor(() => expect(document.querySelectorAll(".layer-row").length).toBeGreaterThan(0));
}

/** A change the guard sees: the top layer's eye. */
async function dirty() {
  fireEvent.click(screen.getByRole("button", { name: /(Hide|Show) Sparkles/ }));
  await waitFor(() => expect(screen.getByText("Unsaved")).toBeTruthy());
  // The guard entry and its listener land in the dirty effect after the "Unsaved" commit;
  // wait for them before any history.back(), or Back can race past the guard.
  await waitForStackGuard();
}

describe("the builder's unsaved guard", () => {
  it("Discard on browser Back from a direct load lands on the effects page", async () => {
    // The builder is the first entry: nothing is behind it, so it closes onto the effects page.
    window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
    await mount();
    await builderReady();
    await dirty();

    window.history.back();
    const dialog = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Discard" }));

    await waitFor(() => expect(screen.queryByTestId("stack-builder")).toBeNull());
    expect(window.location.pathname).toBe("/effects/");
    expect(window.location.search).toBe("");

    // No entry behind the effects page reopens the builder: Back leaves it closed.
    window.history.back();
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId("stack-builder")).toBeNull();
    expect(window.location.pathname).toBe("/effects/");
  });

  it("does not carry a Back's leave over to the close key", async () => {
    // Back asked first, the save it asked for got no name, and the naming dialog was cancelled.
    // The close key's Discard after that is the close key's: it lands on the effects page.
    window.history.replaceState({}, "", "/devices/?state=stack");
    window.history.pushState({ canvasRgbStack: "new" }, "", "/effects/?stack=new&state=complete,stack");
    await mount();
    await screen.findByTestId("stack-builder");
    fireEvent.click(await screen.findByRole("button", { name: "Add Solid" }));
    await waitFor(() => expect(screen.getByText("Unsaved")).toBeTruthy());
    await waitForStackGuard();

    window.history.back();
    const dialog = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    const naming = await screen.findByRole("dialog", { name: "Name the stack" });
    fireEvent.click(within(naming).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Name the stack" })).toBeNull());

    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    const prompt = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    fireEvent.click(within(prompt).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(window.location.pathname).toBe("/effects/"));
    await waitFor(() => expect(screen.queryByTestId("stack-builder")).toBeNull());
    expect(window.location.search).toBe("");
  });

  it("Save on browser Back lands exactly one page back", async () => {
    window.history.replaceState({}, "", "/devices/?state=stack");
    window.history.pushState(
      { canvasRgbStack: "Sunset glow" },
      "",
      "/effects/?stack=Sunset%20glow&state=complete,stack",
    );
    await mount();
    await builderReady();
    await dirty();

    window.history.back();
    const dialog = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    expect(window.location.pathname).toBe("/effects/");
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(window.location.pathname).toBe("/devices/"));
    await waitFor(() => expect(screen.queryByTestId("stack-builder")).toBeNull());
  });

  it("Save on Back that saves but does not start says so in a note that stays (#235)", async () => {
    window.history.replaceState({}, "", "/devices/?state=stack");
    window.history.pushState(
      { canvasRgbStack: "Sunset glow" },
      "",
      "/effects/?stack=Sunset%20glow&state=complete,stack",
    );
    await mount();
    await builderReady();
    await dirty();
    const start = vi.spyOn(api, "activateEffect").mockRejectedValueOnce(
      new ApiFailure(503, "service_unavailable", "the engine is busy"),
    );
    try {
      window.history.back();
      const dialog = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
      fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(window.location.pathname).toBe("/devices/"));
      await waitFor(() => expect(screen.queryByTestId("stack-builder")).toBeNull());
      // The banner left with the builder: the note carries the failure instead.
      expect(await screen.findByText(/Sunset glow is saved, but it did not start: .*the engine is busy/)).toBeTruthy();
    } finally {
      start.mockRestore();
    }
  });

  it("asks before a rail click leaves the builder, and Discard goes on", async () => {
    window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
    await mount();
    await builderReady();
    await dirty();

    fireEvent.click(screen.getByRole("link", { name: /^Layout/ }));
    expect(await screen.findByRole("alertdialog", { name: "Unsaved changes" })).toBeTruthy();
    expect(push).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/"));
  });
});
