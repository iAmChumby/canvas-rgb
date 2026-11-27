import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { RESTART_WAIT_MS, SafeStartBanner, StandingNotices } from "@/shell/GlobalBanners";
import { patchSnap } from "@/lib/store";
import { problems } from "@/lib/status";
import type { SafeStart } from "@/proto/SafeStart";
import type { Status } from "@/proto/Status";

const { pathRef, setSafeStart } = vi.hoisted(() => ({ pathRef: { current: "/" }, setSafeStart: vi.fn() }));
vi.mock("next/navigation", () => ({
  usePathname: () => pathRef.current,
  useRouter: () => ({ push: () => {}, replace: () => {} }),
}));
vi.mock("@/lib/endpoints", () => ({ api: { setSafeStart } }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
beforeAll(() => {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});
beforeEach(() => {
  pathRef.current = "/";
  setSafeStart.mockReset();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function status(safe: SafeStart | null, over: Partial<Status> = {}): Status {
  return {
    active_scene: { id: "safe-start", name: "Safe start" },
    conflicts: [],
    show_zone_panel: false,
    active_effects: ["breathing"],
    safe_start: safe,
    ...over,
  } as unknown as Status;
}

const base: SafeStart = {
  active: true,
  reason: "unclean_once",
  unclean_streak: 1,
  since: null,
  fans_on_bios: true,
};

function show(safe: SafeStart | null, over: Partial<Status> = {}) {
  patchSnap({ status: status(safe, over) });
  return render(<StandingNotices />);
}

describe("the safe-start banner (#577)", () => {
  it("stands as short lines, the body before the actions", () => {
    show(base);
    expect(screen.getByText("Started in safe start")).toBeTruthy();
    expect(screen.getByText("The last run did not end cleanly.")).toBeTruthy();
    expect(
      screen.getByText("Off until you start normally: your saved scene, effects, and display and peripheral readings."),
    ).toBeTruthy();
    expect(screen.getByText("Lights: Breathing.")).toBeTruthy();
    const fans = screen.getByText("Fans: on BIOS control.");
    const button = screen.getByRole("button", { name: "Start normally" });
    expect(fans.compareDocumentPosition(button) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByRole("link", { name: "Help" }).getAttribute("href")).toMatch(/^\/help\/?$/);
    expect(screen.queryByRole("button", { name: /dismiss/i })).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("names a crash loop with the count, and fans not yet confirmed", () => {
    show({ ...base, reason: "crash_loop", unclean_streak: 3, fans_on_bios: false });
    expect(screen.getByText("The last 3 runs did not end cleanly.")).toBeTruthy();
    expect(screen.getByText("Fans: not confirmed on BIOS yet.")).toBeTruthy();
  });

  it("says the owner chose it, and an unknown fan state", () => {
    show({ ...base, reason: "user_chose", fans_on_bios: null });
    expect(screen.getByText("You chose safe start.")).toBeTruthy();
    expect(screen.getByText("Fans: state unknown.")).toBeTruthy();
  });

  it("gives no reason when the service sent none", () => {
    show({ ...base, reason: null });
    expect(screen.queryByText(/did not end cleanly|You chose/)).toBeNull();
  });

  it("takes the fallback colour's name from the status", () => {
    show(base, {
      active_effects: ["solid"],
      active_scene: { id: "safe-start", name: "One steady colour, teal" },
    });
    expect(screen.getByText("Lights: one steady colour, teal.")).toBeTruthy();
  });

  it("claims no light before one is playing", () => {
    show(base, { active_effects: [] });
    expect(screen.getByText("Lights: not started yet.")).toBeTruthy();
  });

  it("shows the restart while it is out", async () => {
    setSafeStart.mockResolvedValue({ mode: "normal", restarting: true });
    show(base);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Start normally" }));
    });
    expect(setSafeStart).toHaveBeenCalledWith("normal");
    expect(screen.getByText("Restarting in normal mode…")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Start normally" }).getAttribute("aria-busy")).toBe("true");
  });

  it("puts a refusal on its own alert line", async () => {
    setSafeStart.mockRejectedValue(new Error("the service is not answering"));
    show(base);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Start normally" }));
    });
    expect(screen.getByRole("alert").textContent).toContain("the service is not answering");
    expect(screen.queryByText("Restarting in normal mode…")).toBeNull();
  });

  it("gives the key back when the restart never comes", async () => {
    vi.useFakeTimers();
    setSafeStart.mockResolvedValue({ mode: "normal", restarting: true });
    show(base);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Start normally" }));
    });
    await act(async () => {
      vi.advanceTimersByTime(RESTART_WAIT_MS);
    });
    expect(screen.getByRole("alert").textContent).toContain("did not restart");
    expect(screen.queryByText("Restarting in normal mode…")).toBeNull();
  });

  it("leaves Help & About to draw it, with no key and no link there", () => {
    pathRef.current = "/help/";
    show(base);
    expect(screen.queryByText("Started in safe start")).toBeNull();
    cleanup();
    render(<SafeStartBanner onHelp />);
    expect(screen.getByText("Started in safe start")).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.queryByRole("link")).toBeNull();
  });

  it("stands on nothing when the mode is normal", () => {
    show(null);
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
