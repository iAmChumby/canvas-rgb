import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { fixtureState } from "@/lib/demoUpdate";
import { SAMPLE_SOURCE } from "@/lib/demoEffects";
import { patchSnap, refreshAll } from "@/lib/store";
import { GlobalBanners } from "@/shell/GlobalBanners";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { Rail } from "@/shell/Rail";
import { EffectsView } from "@/views/EffectsView";

// Unsaved code survives no page change by accident (D2.2): the rail asks the editor's guard
// first, and the editor registers it only while its source has unsaved changes.

const { push } = vi.hoisted(() => ({ push: vi.fn() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push, replace: vi.fn(), prefetch: vi.fn(), back: vi.fn() }),
  usePathname: () => "/effects/",
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

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=html");
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
  const view = render(
    <ToastProvider>
      <ToastHost />
      <GlobalBanners />
      <Rail pathname="/effects/" />
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>
    </ToastProvider>,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Open Sample HTML" }));
  await screen.findByRole("heading", { name: "Sample HTML" });
  fireEvent.click(screen.getByRole("button", { name: "Edit code" }));
  await screen.findByTestId("effect-code");
  return view;
}

async function replaceSource(text: string) {
  const content = document.querySelector(".cm-content") as HTMLElement;
  content.focus();
  fireEvent.keyDown(content, { key: "a", code: "KeyA", keyCode: 65, ctrlKey: true });
  await act(async () => {
    fireEvent.paste(content, { clipboardData: { getData: () => text } });
  });
}

/** A change that still parses, so the prompt's Save can import it. */
const validChange = () => replaceSource(`${SAMPLE_SOURCE}
<!-- an edit that is not saved yet -->
`);

async function dirty() {
  await replaceSource("<html><!-- a change --></html>");
}

const railLink = () => screen.getByRole("link", { name: /^Layout/ });

describe("leaving the editor for another page", () => {
  it("asks before a rail click leaves unsaved code, and goes on Discard", async () => {
    await mount();
    await dirty();
    fireEvent.click(railLink());
    expect(await screen.findByRole("alertdialog", { name: "Unsaved changes" })).toBeTruthy();
    expect(push).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/"));
  });

  it("navigates at once with nothing to lose, and holds no guard then", async () => {
    await mount();
    fireEvent.click(railLink());
    await waitFor(() => expect(push).toHaveBeenCalledWith("/"));
    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
  });

  it("drops the held page change on Keep editing", async () => {
    await mount();
    await dirty();
    fireEvent.click(railLink());
    fireEvent.click(await screen.findByRole("button", { name: "Keep editing" }));
    expect(push).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
  });

  it("asks before the browser's Back leaves unsaved code, and Discard goes back", async () => {
    window.history.replaceState({}, "", "/devices/?state=html");
    window.history.pushState({}, "", "/effects/?state=html");
    await mount();
    await dirty();
    window.history.back();
    expect(await screen.findByRole("alertdialog", { name: "Unsaved changes" })).toBeTruthy();
    expect(window.location.pathname).toBe("/effects/");
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(window.location.pathname).toBe("/devices/"));
    expect(push).not.toHaveBeenCalled();
  });

  it("goes back with no prompt when nothing is unsaved", async () => {
    window.history.replaceState({}, "", "/devices/?state=html");
    window.history.pushState({}, "", "/effects/?state=html");
    await mount();
    window.history.back();
    await waitFor(() => expect(window.location.pathname).toBe("/devices/"));
    expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull();
    window.history.replaceState({}, "", "/effects/?state=html");
  });

  it("carries the held rail navigation out when the prompt's Save lands", async () => {
    window.history.replaceState({}, "", "/effects/?state=html");
    await mount();
    await validChange();
    fireEvent.click(railLink());
    const dialog = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    push.mockClear();
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/"), { timeout: 5000 });
    expect(push.mock.calls.filter((call) => call[0] === "/").length).toBe(1);
  });

  it("asks before the update banner's Update now leaves unsaved code", async () => {
    window.history.replaceState({}, "", "/effects/?state=html&update=available");
    patchSnap({ update: fixtureState("available") });
    await mount();
    await dirty();
    fireEvent.click(screen.getByRole("button", { name: "Update now" }));
    expect(await screen.findByRole("alertdialog", { name: "Unsaved changes" })).toBeTruthy();
    expect(push).not.toHaveBeenCalledWith("/updates/");
    fireEvent.click(within(screen.getByRole("alertdialog", { name: "Unsaved changes" })).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/updates/"));
    patchSnap({ update: null });
    window.history.replaceState({}, "", "/effects/?state=html");
  });

  it("does not import or take a drop while the editor is open", async () => {
    await mount();
    expect((screen.getByRole("button", { name: "Import" }) as HTMLButtonElement).disabled).toBe(true);
    const imports = ((globalThis as { __imports?: unknown[] }).__imports ??= []);
    imports.length = 0;
    const page = document.querySelector(".rd-effects") as HTMLElement;
    fireEvent.drop(page, {
      dataTransfer: { files: [new File(["<html></html>"], "drop.html", { type: "text/html" })] },
    });
    await new Promise((resolve) => setTimeout(resolve, 80));
    expect(imports.length).toBe(0);
    expect(screen.queryByText(/Drop an .html effect/)).toBeNull();
  });

  it("warns the browser only while the source has unsaved changes", async () => {
    await mount();
    const clean = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(clean);
    expect(clean.defaultPrevented).toBe(false);
    await dirty();
    const changed = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(changed);
    expect(changed.defaultPrevented).toBe(true);
    // Revert puts the source back: the browser is no longer warned.
    fireEvent.click(screen.getByRole("button", { name: "Revert" }));
    const after = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(after);
    expect(after.defaultPrevented).toBe(false);
  });
});
