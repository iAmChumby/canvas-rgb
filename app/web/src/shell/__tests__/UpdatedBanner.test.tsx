import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getSnap, patchSnap } from "@/lib/store";
import { fixtureState } from "@/lib/demoUpdate";
import { GlobalBanners } from "@/shell/GlobalBanners";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { UpdatesView } from "@/views/UpdatesView";

// #125: the "Updated to" banner stayed over the page until the × was pressed, and sat over the
// release notes on the What's new page itself.
//
// The banner is expected to behave like the app's toasts, and to count a visit to What's new as
// seen. The fixture form of the dismiss is CMD_UPDATE_DISMISS_UPDATED in lib/demoUpdate.ts, which
// is the same command the banner's × sends through lib/host.ts; the test reads the result out of
// the store, not out of the fixture, so it exercises the whole path.

// The action-toast timeout (components/Toast.tsx, ACTION_LIFETIME_MS). The banner carries an
// action ("What's new"), so it reuses the action toast's timeout. Kept as a local constant so this
// file also loads against the old code, which had neither the timer nor the export.
const TOAST_ACTION_MS = 7000;
// The leaving transition's tail, the same one Toast.tsx waits before removing the toast.
const LEAVE_TAIL_MS = 200;

const nav = vi.hoisted(() => ({ pathname: "/", push: vi.fn() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: nav.push, replace: vi.fn(), prefetch: vi.fn(), back: vi.fn() }),
  usePathname: () => nav.pathname,
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
        nav.push(href);
      }}
    >
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
  patchSnap({ update: null });
  nav.push.mockClear();
  vi.useRealTimers();
  nav.pathname = "/";
  window.history.replaceState({}, "", "/");
});

/** The floating notices' own box; the empty .banners wrapper stays in the DOM. */
const bannerBox = () => document.querySelector(".banners")?.textContent ?? "";
const showsUpdatedBanner = () => bannerBox().includes("Updated to");

function renderBanner() {
  return render(<GlobalBanners />);
}

describe("the updated-to banner", () => {
  it("is not drawn on the Updates page, even while updated_to is still set", async () => {
    window.history.replaceState({}, "", "/updates/?update=updated");
    nav.pathname = "/updates/";
    patchSnap({ update: fixtureState("updated") });
    renderBanner();
    await act(async () => {});
    expect(showsUpdatedBanner()).toBe(false);
  });

  it("leaves when the person goes to any other page", async () => {
    vi.useFakeTimers();
    window.history.replaceState({}, "", "/?update=updated");
    nav.pathname = "/";
    patchSnap({ update: fixtureState("updated") });
    const view = renderBanner();
    expect(showsUpdatedBanner()).toBe(true);

    nav.pathname = "/devices/";
    view.rerender(<GlobalBanners />);
    act(() => {
      vi.advanceTimersByTime(LEAVE_TAIL_MS + 150);
    });
    expect(showsUpdatedBanner()).toBe(false);
  });

  it("goes on its own after the toast timeout", async () => {
    vi.useFakeTimers();
    window.history.replaceState({}, "", "/?update=updated");
    nav.pathname = "/";
    patchSnap({ update: fixtureState("updated") });
    renderBanner();
    expect(showsUpdatedBanner()).toBe(true);

    act(() => {
      vi.advanceTimersByTime(TOAST_ACTION_MS);
    });
    // The leaving fade plays, then the host drops the banner.
    act(() => {
      vi.advanceTimersByTime(LEAVE_TAIL_MS);
    });
    expect(showsUpdatedBanner()).toBe(false);
  });

  it("counts opening the Updates page as seen: updated_to is dismissed through the same command the x uses", async () => {
    window.history.replaceState({}, "", "/updates/?update=updated");
    nav.pathname = "/updates/";
    patchSnap({ update: fixtureState("updated") });

    render(
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <UpdatesView />
      </StripSlotsProvider>,
    );
    await act(async () => {});

    expect(getSnap().update?.updated_to).toBeNull();
    // The release notes stay readable for this visit.
    expect(screen.getByText("Added")).toBeTruthy();
  });
});
