import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NotifyHost, dismissNotifications } from "@/components/Notify";
import { continueNavigation, setNavigationGuard } from "@/lib/navigationGuard";
import { getSnap, patchSnap } from "@/lib/store";
import { fixtureState } from "@/lib/demoUpdate";
import { GlobalBanners } from "@/shell/GlobalBanners";
import { OnboardingGate } from "@/views/editor/OnboardingGate";
import type { Status } from "@/proto/Status";
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

const nav = vi.hoisted(() => ({ pathname: "/", push: vi.fn(), replace: vi.fn() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: nav.push, replace: nav.replace, prefetch: vi.fn(), back: vi.fn() }),
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
  patchSnap({ update: null, status: null });
  nav.push.mockClear();
  nav.replace.mockReset();
  vi.useRealTimers();
  nav.pathname = "/";
  window.history.replaceState({}, "", "/");
});

const baseStatus: Status = {
  version: "0.10.1",
  power: "running",
  brightness: 100,
  fps_target: 30,
  fps_measured: 30,
  frame_time_ms: 33.3,
  frames_rendered: 0,
  parked: false,
  devices_connected: 3,
  devices_total: 4,
  preview_subscribers: 0,
  active_layout: "desk",
  active_effect: null,
  active_effects: [],
  openrgb: "connected",
  service: "connected",
  service_down_ms: null,
  conflicts: [],
  show_zone_panel: false,
  onboarding_complete: true,
};

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

/** A swipe to the right across a notice: the library's own gesture (pointer down, move, up). */
function swipe(target: HTMLElement) {
  const at = (type: string, clientX: number) =>
    fireEvent(
      target,
      new MouseEvent(type, { bubbles: true, cancelable: true, clientX, clientY: 10, button: 0 }),
    );
  // jsdom has no pointer capture; the library asks for it on the first touch.
  (target as unknown as { setPointerCapture: () => void }).setPointerCapture = () => undefined;
  at("pointerdown", 10);
  // The library locks the direction on the first move and follows the finger on the next.
  at("pointermove", 30);
  at("pointermove", 130);
  vi.advanceTimersByTime(20);
  at("pointerup", 130);
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

  it("goes on its own at the action-toast timeout, and leaves updated_to set", () => {
    mountUpdated();
    settle();
    act(() => {
      vi.advanceTimersByTime(TOAST_ACTION_MS - 100);
    });
    expect(laneNotice()).not.toBeNull();
    expect(laneNotice()?.getAttribute("data-removed")).not.toBe("true");
    // The clock has run out 50 ms past the timeout: the library has the notice on its way out.
    act(() => {
      vi.advanceTimersByTime(150);
    });
    const leaving = laneNotice();
    expect(leaving === null || leaving.getAttribute("data-removed") === "true").toBe(true);
    drain();
    expect(laneNotice()).toBeNull();
    // A timeout is this session only: the release is still marked until the person sees it.
    expect(getSnap().update?.updated_to).not.toBeNull();
  });

  it("waits while the pointer is on it, and goes a life after the pointer leaves", () => {
    mountUpdated();
    settle();
    const notice = laneNotice() as HTMLElement;
    fireEvent.mouseEnter(notice.querySelector(".notify-body") as HTMLElement);
    // Held well past its life.
    act(() => {
      vi.advanceTimersByTime(TOAST_ACTION_MS + 3000);
    });
    expect(laneNotice()).not.toBeNull();
    expect(laneNotice()?.getAttribute("data-removed")).not.toBe("true");
    fireEvent.mouseLeave(laneNotice()?.querySelector(".notify-body") as HTMLElement);
    // The life that was left when the pointer arrived starts again.
    act(() => {
      vi.advanceTimersByTime(TOAST_ACTION_MS - 100);
    });
    expect(laneNotice()?.getAttribute("data-removed")).not.toBe("true");
    drain();
    expect(laneNotice()).toBeNull();
    expect(getSnap().update?.updated_to).not.toBeNull();
  });

  it("leaves when the person goes to any other page, and leaves updated_to set", () => {
    const view = mountUpdated();
    settle();
    expect(laneNotice()).not.toBeNull();
    nav.pathname = "/devices/";
    view.rerender(<Shelf />);
    drain();
    expect(laneNotice()).toBeNull();
    // Going is this session only: the release stays marked until What's new or the close key.
    expect(getSnap().update?.updated_to).not.toBeNull();
    // A re-render does not bring it back.
    patchSnap({ update: { ...getSnap().update!, last_check: "2026-11-09T00:00:00Z" } });
    view.rerender(<Shelf />);
    settle();
    expect(laneNotice()).toBeNull();
  });

  it("a re-render on the same page does not fire a second notice", () => {
    const view = mountUpdated();
    settle();
    patchSnap({ update: { ...getSnap().update!, last_check: "2026-11-09T00:00:00Z" } });
    view.rerender(<Shelf />);
    settle();
    expect(document.querySelectorAll("[data-sonner-toast]").length).toBe(1);
  });

  it("a swipe away counts as the close key: updated_to is dismissed", async () => {
    mountUpdated();
    settle();
    const notice = laneNotice() as HTMLElement;
    swipe(notice);
    // The dismiss is a command to the tray: its answer lands a tick later.
    await act(async () => {});
    drain();
    expect(laneNotice()).toBeNull();
    expect(getSnap().update?.updated_to).toBeNull();
  });

  it("on the Updates page every notice a second release added goes, not only the last", () => {
    const view = mountUpdated();
    settle();
    const first = getSnap().update!;
    // The first notice is held by the pointer, so it is still up when the second release lands.
    fireEvent.mouseEnter(laneNotice()?.querySelector(".notify-body") as HTMLElement);
    patchSnap({ update: { ...first, updated_to: { ...first.updated_to!, version: "9.99.0" } } });
    view.rerender(<Shelf />);
    settle();
    const titles = () =>
      Array.from(document.querySelectorAll("[data-sonner-toast]")).map((n) => n.textContent ?? "");
    expect(titles().filter((t) => t.includes("Updated to")).length).toBe(2);

    nav.pathname = "/updates/";
    view.rerender(<Shelf />);
    drain();
    expect(titles().filter((t) => t.includes("Updated to")).length).toBe(0);
  });

  it("What's new while the unsaved-changes prompt holds the page keeps the notice until the page goes", () => {
    const view = mountUpdated();
    settle();
    setNavigationGuard(() => false);
    try {
      fireEvent.click(within(laneNotice() as HTMLElement).getByRole("button", { name: /What.s new/ }));
      // The page did not change: the guard holds it, and so the notice stays.
      expect(nav.push).not.toHaveBeenCalled();
      drain();
      expect(laneNotice()).not.toBeNull();
    } finally {
      setNavigationGuard(null);
    }
    // The person lets the page go: the held navigation runs, and the arrival takes the notice.
    act(() => {
      continueNavigation();
    });
    expect(nav.push).toHaveBeenCalledWith("/updates/");
    nav.pathname = "/updates/";
    view.rerender(<Shelf />);
    drain();
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

  it("a held What's new pins the notice: the pointer leaving does not start its life again", () => {
    const view = mountUpdated();
    settle();
    setNavigationGuard(() => false);
    try {
      const notice = laneNotice() as HTMLElement;
      const body = notice.querySelector(".notify-body") as HTMLElement;
      fireEvent.mouseEnter(body);
      fireEvent.click(within(notice).getByRole("button", { name: /What.s new/ }));
      expect(nav.push).not.toHaveBeenCalled();
      fireEvent.mouseLeave(body);
      // Well past its life, with the prompt still up: the notice is still there.
      act(() => {
        vi.advanceTimersByTime(TOAST_ACTION_MS + 3000);
      });
      drain();
      expect(laneNotice()).not.toBeNull();
      expect(laneNotice()?.getAttribute("data-removed")).not.toBe("true");
      expect(nav.push).not.toHaveBeenCalled();
    } finally {
      setNavigationGuard(null);
    }
    // The person lets the page go: the held navigation runs, and the arrival takes the notice.
    act(() => {
      continueNavigation();
    });
    expect(nav.push).toHaveBeenCalledWith("/updates/");
    nav.pathname = "/updates/";
    view.rerender(<Shelf />);
    drain();
    expect(laneNotice()).toBeNull();
  });

  it("the onboarding gate's first-launch redirect is the app's page change, not the person's: the notice stays", () => {
    // A launch after an update by someone who put the setup off: both the notice and the redirect.
    window.sessionStorage.clear();
    nav.pathname = "/effects/";
    window.history.replaceState({}, "", "/effects/?update=updated");
    patchSnap({ update: updatedState() });
    patchSnap({ status: { ...baseStatus, onboarding_complete: false } });
    nav.replace.mockImplementation(() => {
      nav.pathname = "/";
    });
    const view = render(
      <>
        <Shelf />
        <OnboardingGate />
      </>,
    );
    settle();
    expect(laneNotice()).not.toBeNull();
    // The gate changed the page by itself.
    expect(nav.replace).toHaveBeenCalledWith("/");
    view.rerender(
      <>
        <Shelf />
        <OnboardingGate />
      </>,
    );
    // Long enough for a dismissal to finish, short of the toast's own life.
    drain();
    expect(laneNotice()).not.toBeNull();
    expect(laneNotice()?.getAttribute("data-removed")).not.toBe("true");
    // The person's own first navigation afterwards still sends it.
    nav.pathname = "/devices/";
    view.rerender(
      <>
        <Shelf />
        <OnboardingGate />
      </>,
    );
    drain();
    expect(laneNotice()).toBeNull();
    patchSnap({ status: null });
    window.sessionStorage.clear();
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
