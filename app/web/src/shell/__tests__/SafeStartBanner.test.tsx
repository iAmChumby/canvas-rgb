import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { StandingNotices } from "@/shell/GlobalBanners";
import { patchSnap } from "@/lib/store";
import { problems } from "@/lib/status";
import type { SafeStart } from "@/proto/SafeStart";
import type { Status } from "@/proto/Status";

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
afterEach(cleanup);

function status(safe: SafeStart | null): Status {
  return {
    active_scene: { id: "everything", name: "Everything" },
    conflicts: [],
    show_zone_panel: false,
    active_effects: ["breathing"],
    safe_start: safe,
  } as unknown as Status;
}

const base: SafeStart = {
  active: true,
  reason: "unclean_once",
  unclean_streak: 1,
  since: null,
  fans_on_bios: true,
};

describe("the safe-start banner (#577)", () => {
  it("stands with the reason, what is off, the lights and the fan answer", () => {
    patchSnap({ status: status(base) });
    render(<StandingNotices />);
    expect(screen.getByText("Started in safe start")).toBeTruthy();
    expect(screen.getByText(/The last run did not end cleanly/)).toBeTruthy();
    expect(screen.getByText(/Lights run Breathing/)).toBeTruthy();
    expect(screen.getByText(/Fans are on BIOS control/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Start normally" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Help & About" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /dismiss/i })).toBeNull();
  });

  it("names a crash loop with the count and a choice the owner made", () => {
    patchSnap({ status: status({ ...base, reason: "crash_loop", unclean_streak: 3, fans_on_bios: false }) });
    render(<StandingNotices />);
    expect(screen.getByText(/The last 3 runs did not end cleanly/)).toBeTruthy();
    expect(screen.getByText(/Fans are not confirmed on BIOS/)).toBeTruthy();
  });

  it("says the owner chose it", () => {
    patchSnap({ status: status({ ...base, reason: "user_chose", fans_on_bios: null }) });
    render(<StandingNotices />);
    expect(screen.getByText(/You started safe start/)).toBeTruthy();
    expect(screen.getByText(/Fan state is unknown/)).toBeTruthy();
  });

  it("stands on nothing when the mode is normal", () => {
    patchSnap({ status: status(null) });
    render(<StandingNotices />);
    expect(screen.queryByText("Started in safe start")).toBeNull();
  });
});

describe("the strip mark (#577)", () => {
  it("says Safe start, with the short word for a phone", () => {
    expect(problems(status(base), null)[0]).toEqual({
      tone: "warn",
      text: "Safe start",
      short: "Safe",
    });
  });

  it("is first, ahead of a link that is down", () => {
    const down = { ...status(base), openrgb: "down" } as Status;
    expect(problems(down, null).map((p) => p.text)).toEqual(["Safe start", "OpenRGB down"]);
  });
});
