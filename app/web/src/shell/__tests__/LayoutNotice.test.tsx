import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { StandingNotices } from "@/shell/GlobalBanners";
import { patchSnap } from "@/lib/store";
import type { Status } from "@/proto/Status";

// The standing notices: the SignalRGB conflict, the zone panel and the "there is no layout yet"
// notice. The Layout page is where a layout is made, so the notice does not stand there: it
// would say it twice and push the page down when it arrives.

const { pathRef } = vi.hoisted(() => ({ pathRef: { current: "/" } }));
vi.mock("next/navigation", () => ({
  usePathname: () => pathRef.current,
  useRouter: () => ({ push: () => {}, replace: () => {} }),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

afterEach(() => {
  cleanup();
});

/** A status with no layout, which is what makes the notice stand. */
function noLayout(): Status {
  return {
    active_layout: null,
    conflicts: [],
    show_zone_panel: false,
  } as unknown as Status;
}

describe("the notice that there is no layout yet", () => {
  it("does not stand on the Layout page: that page says it in its own frame", () => {
    pathRef.current = "/";
    patchSnap({ status: noLayout() });
    render(<StandingNotices />);
    expect(screen.queryByText("There is no layout yet")).toBeNull();
    expect(screen.queryByText("Open the Layout")).toBeNull();
  });

  it("stands on the other pages, with the way to the Layout", () => {
    pathRef.current = "/effects/";
    patchSnap({ status: noLayout() });
    render(<StandingNotices />);
    expect(screen.getByText("There is no layout yet")).toBeTruthy();
    expect(screen.getByText("Open the Layout")).toBeTruthy();
  });

  it("stands on no page while there is a layout", () => {
    pathRef.current = "/effects/";
    patchSnap({ status: { ...noLayout(), active_layout: "desk" } });
    render(<StandingNotices />);
    expect(screen.queryByText("There is no layout yet")).toBeNull();
  });
});
