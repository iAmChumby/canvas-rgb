import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
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
    <ToastProvider>
      <ToastHost />
      <Rail pathname="/effects/" />
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>
    </ToastProvider>,
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
