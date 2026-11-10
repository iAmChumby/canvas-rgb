import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NotifyHost, dismissNotifications } from "@/components/Notify";
import { getSnap, patchSnap } from "@/lib/store";
import { fixtureState } from "@/lib/demoUpdate";
import { GlobalBanners } from "@/shell/GlobalBanners";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { UpdatesView } from "@/views/UpdatesView";

// #125 and #168: the "Updated to" notice is a toast. It speaks through the notification lane
// (Sonner, behind components/Notify.tsx), not through a banner of its own with hand-rolled
// timers. It leaves on its own after the action-toast timeout, is not shown on the Updates page,
// counts a visit to What's new as seen, and fires once per release per session.
//
// The fixture form of the dismiss is CMD_UPDATE_DISMISS_UPDATED in lib/demoUpdate.ts, which is
// the same command the close key sends through lib/host.ts; the tests read the result out of the
// store, not out of the fixture, so they exercise the whole path.

// The toast's own life (shell/UpdatedToast.tsx, UPDATED_TOAST_MS). A local constant, so this file
// also loads against the old code, which had no such export.
const TOAST_ACTION_MS = 7000;

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

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  act(() => {
    dismissNotifications();
    vi.advanceTimersByTime(600);
  });
  cleanup();
  patchSnap({ update: null });
  nav.push.mockClear();
  vi.useRealTimers();
  nav.pathname = "/";
  window.history.replaceState({}, "", "/");
});

/** The shell's two hosts: the banners' mount and the notification lane. */
function Shelf() {
  return (
    <>
      <GlobalBanners />
      <NotifyHost />
    </>
  );
}

/** Lets the library publish a message (it does so on the next tick). */
function settle(ms = 1) {
  act(() => {
    vi.advanceTimersByTime(ms);
  });
}

/** Lets the library take a message away: the hand-off runs in rounds of frames and flushes. */
function drain() {
  for (let i = 0; i < 12; i++) settle(500);
}

/** The notice inside the lane: null when the lane does not carry it. */
const laneNotice = () =>
  Array.from(document.querySelectorAll("[data-sonner-toaster] [data-sonner-toast]")).find((n) =>
    (n.textContent ?? "").includes("Updated to"),
  ) ?? null;
/** The old banner's mount: the notice must not be drawn there. */
const bannerNotice = () => (document.querySelector(".banners")?.textContent ?? "").includes("Updated to");

// Each test installs its own release number: the toast fires once per release per session, so a
// number shared between tests would be spent by the first of them.
let release = 0;
/** The "updated" fixture, as a release of its own. */
function updatedState() {
  const state = fixtureState("updated")!;
  return { ...state, updated_to: { ...state.updated_to!, version: `9.${++release}.0` } };
}
const versionOf = () => getSnap().update?.updated_to?.version;

function mountUpdated(path = "/") {
  window.history.replaceState({}, "", `${path}?update=updated`);
  nav.pathname = path;
  patchSnap({ update: updatedState() });
  return render(<Shelf />);
}

describe("the updated-to toast", () => {
  it("is a toast in the lane, with the version, the current version and the What's new key", () => {
    mountUpdated();
    settle();
    const notice = laneNotice();
    expect(notice).not.toBeNull();
    expect(notice?.textContent).toContain(`Updated to ${versionOf()}`);
    expect(notice?.textContent).toContain(`You have ${getSnap().update?.current_version}.`);
    expect(within(notice as HTMLElement).getByRole("button", { name: /What.s new/ })).toBeTruthy();
    expect(within(notice as HTMLElement).getByRole("button", { name: "Dismiss" })).toBeTruthy();
    // The hand-rolled banner is gone.
    expect(bannerNotice()).toBe(false);
  });

  it("is not drawn on the Updates page, even while updated_to is still set", () => {
    mountUpdated("/updates/");
    settle();
    expect(laneNotice()).toBeNull();
    expect(bannerNotice()).toBe(false);
  });

  it("goes away when the person arrives on the Updates page", () => {
    const view = mountUpdated();
    settle();
    expect(laneNotice()).not.toBeNull();
    nav.pathname = "/updates/";
    view.rerender(<Shelf />);
    drain();
    expect(laneNotice()).toBeNull();
  });

  it("goes on its own after the action-toast timeout, and leaves updated_to set", () => {
    mountUpdated();
    settle();
    act(() => {
      vi.advanceTimersByTime(TOAST_ACTION_MS - 100);
    });
    expect(laneNotice()).not.toBeNull();
    drain();
    expect(laneNotice()).toBeNull();
    // A timeout is this session only: the release is still marked until the person sees it.
    expect(getSnap().update?.updated_to).not.toBeNull();
  });

  it("stays through navigation and a re-render without firing a second time", () => {
    const view = mountUpdated();
    settle();
    nav.pathname = "/devices/";
    view.rerender(<Shelf />);
    patchSnap({ update: { ...getSnap().update!, last_check: "2026-11-09T00:00:00Z" } });
    view.rerender(<Shelf />);
    settle();
    expect(document.querySelectorAll("[data-sonner-toast]").length).toBe(1);
    // Gone, and not brought back by the next re-render.
    for (let i = 0; i < 20; i++) settle(500);
    expect(laneNotice()).toBeNull();
    view.rerender(<Shelf />);
    settle();
    expect(laneNotice()).toBeNull();
  });

  it("the close key dismisses updated_to through the command the Updates page uses", async () => {
    mountUpdated();
    settle();
    const notice = laneNotice() as HTMLElement;
    await act(async () => {
      fireEvent.click(within(notice).getByRole("button", { name: "Dismiss" }));
    });
    drain();
    expect(getSnap().update?.updated_to).toBeNull();
    expect(laneNotice()).toBeNull();
  });

  it("What's new opens the Updates page", () => {
    mountUpdated();
    settle();
    const notice = laneNotice() as HTMLElement;
    fireEvent.click(within(notice).getByRole("button", { name: /What.s new/ }));
    expect(nav.push).toHaveBeenCalledWith("/updates/");
  });

  it("counts opening the Updates page as seen: updated_to is dismissed through the same command the x uses", async () => {
    window.history.replaceState({}, "", "/updates/?update=updated");
    nav.pathname = "/updates/";
    patchSnap({ update: updatedState() });

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
