import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
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
      <Rail pathname="/effects/" />
      <EffectsView />
    </ToastProvider>,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Open Sample HTML" }));
  await screen.findByRole("heading", { name: "Sample HTML" });
  fireEvent.click(screen.getByRole("button", { name: "Edit code" }));
  await screen.findByTestId("effect-code");
  return view;
}

async function dirty() {
  const content = document.querySelector(".cm-content") as HTMLElement;
  content.focus();
  fireEvent.keyDown(content, { key: "a", code: "KeyA", keyCode: 65, ctrlKey: true });
  await act(async () => {
    fireEvent.paste(content, { clipboardData: { getData: () => "<html><!-- a change --></html>" } });
  });
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
