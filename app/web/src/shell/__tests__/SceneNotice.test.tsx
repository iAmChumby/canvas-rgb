import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { StandingNotices } from "@/shell/GlobalBanners";
import { patchSnap } from "@/lib/store";
import type { Status } from "@/proto/Status";

// The standing notices: the SignalRGB conflict, the zone panel and the "there is no scene yet"
// notice. The Scenes page is where scenes are, so the notice does not stand there: it
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

/** A status with no scene, which is what makes the notice stand. */
function noScene(): Status {
  return {
    active_scene: null,
    conflicts: [],
    show_zone_panel: false,
  } as unknown as Status;
}

describe("the notice that there is no scene yet", () => {
  it("does not stand on the Scenes page: that page says it in its own frame", () => {
    pathRef.current = "/";
    patchSnap({ status: noScene() });
    render(<StandingNotices />);
    expect(screen.queryByText("There is no scene yet")).toBeNull();
    expect(screen.queryByText("Open Scenes")).toBeNull();
  });

  it("stands on the other pages, with the way to Scenes", () => {
    pathRef.current = "/effects/";
    patchSnap({ status: noScene() });
    render(<StandingNotices />);
    expect(screen.getByText("There is no scene yet")).toBeTruthy();
    expect(screen.getByText("Open Scenes")).toBeTruthy();
  });

  it("stands on no page while there is a scene", () => {
    pathRef.current = "/effects/";
    patchSnap({ status: { ...noScene(), active_scene: { id: "gaming", name: "Gaming" } } });
    render(<StandingNotices />);
    expect(screen.queryByText("There is no scene yet")).toBeNull();
  });
});
