// The chain takeover: the first run's offer, one header after another, and the two ways it stays
// shut (Later for the rest of the launch, an empty list of headers).
//
// The daemon is the mocked chain routes, answered the way the other chain tests answer them, so
// what is checked is the takeover against the store it reads and the flow it draws: which header
// it offers, what closing one does to the next, and what Escape leaves behind.

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const { api } = vi.hoisted(() => ({
  api: {
    chains: {
      list: vi.fn(),
      start: vi.fn(),
      light: vi.fn(),
      cancel: vi.fn(),
      empty: vi.fn(),
      commit: vi.fn(),
    },
    lights: { list: vi.fn() },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

const { nav } = vi.hoisted(() => ({ nav: { path: "/" } }));
vi.mock("next/navigation", () => ({
  usePathname: () => nav.path,
  useRouter: () => ({ push: () => {}, replace: () => {}, prefetch: () => {} }),
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

import { patchSnap } from "@/lib/store";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { ChainSetupState } from "@/proto/ChainSetupState";
import type { ChainView } from "@/proto/ChainView";
import type { LightView } from "@/proto/LightView";
import type { Status } from "@/proto/Status";
import { StandingNotices } from "@/shell/GlobalBanners";
import { ChainTakeover } from "@/views/chain/ChainTakeover";
import { chainLaterThisLaunch } from "@/views/editor/flow";
import { cancelNavigation, hasPendingNavigation, setNavigationGuard } from "@/lib/navigationGuard";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
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
});

const DEVICE = "openrgb:A1B2C3D4E5F6";
const SESSION = "session-1";
const SETUP = { name: "Light setup" };

const STATUS = {
  onboarding_complete: true,
  show_zone_panel: false,
  conflicts: [],
  active_scene: "gaming",
  openrgb: "connected",
} as unknown as Status;

/** One part of a set-up chain, by position (1-based), at `start` along the zone. */
function part(position: number, name: string, leds: number, start: number, kind: ChainPartKind): LightView {
  return {
    light: {
      id: `${DEVICE}#z1p${position}`,
      device_id: DEVICE,
      zone_index: 1,
      leds: { start, count: leds },
      name,
      arrangement: kind === "fan" ? "ring" : "strip",
      reversed: false,
      kind,
    },
    led_count: leds,
    connected: true,
    label: name,
  };
}

/**
 * One resizable header as the daemon lists it. `leds` is the length it has now: 0 means the app
 * does not know how long the header is. A size is not a chain record: a header with a size but no
 * record is unverified (a saved size re-applied at start-up) and the takeover offers it too. Only
 * a header with a record (set up or empty) is never offered again (#380, #470).
 */
function chain(zone: number, name: string, state: ChainSetupState, parts: LightView[] = [], leds = 0): ChainView {
  return {
    device_id: DEVICE,
    device_name: "MSI MYSTIC LIGHT",
    zone_index: zone,
    zone_name: name,
    state,
    leds,
    min_leds: 0,
    max_leds: 300,
    parts,
  };
}

/** Mounts the takeover with the list the daemon answers, and lets the reads settle. */
async function show(chains: ChainView[] | null, status: Status = STATUS) {
  api.chains.list.mockResolvedValue({ chains: chains ?? [] });
  patchSnap({ status, chains });
  const view = render(<ChainTakeover />);
  await flush();
  return view;
}

/** Lets pending answers land and moves the clock by `ms`. */
async function flush(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** Clicks a key and lets what it sends settle. */
async function press(name: string): Promise<void> {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
    await vi.advanceTimersByTimeAsync(0);
  });
}

/**
 * Walks the flow on screen from the intro to the save: Start, the three Nexts (count, slide,
 * devices) and the save key.
 */
async function finish(): Promise<void> {
  await press("Start");
  await press("Next");
  await press("Next");
  await press("Next");
  await press("Yes, save these devices");
}

describe("the chain takeover", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    for (const fn of Object.values(api.chains)) fn.mockReset();
    api.lights.list.mockReset();
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 120, raised_to: 300 });
    api.chains.light.mockResolvedValue({ ok: true });
    api.chains.cancel.mockResolvedValue({ ok: true });
    api.chains.empty.mockResolvedValue({ ok: true });
    api.chains.commit.mockResolvedValue({ lights: [], final_leds: 120 });
    api.chains.list.mockResolvedValue({ chains: [] });
    api.lights.list.mockResolvedValue({ lights: [] });
    window.localStorage.clear();
    window.sessionStorage.clear();
    patchSnap({ chains: null, lights: null });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("opens by itself when onboarding is complete and a chain is not set up", async () => {
    await show([chain(1, "JARGB 1", "not_set_up")]);

    expect(api.chains.list).toHaveBeenCalled();
    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    expect(screen.getByText(/MSI MYSTIC LIGHT · JARGB 1/)).toBeTruthy();
    // The introduction comes first: nothing on the hardware changes until Start.
    expect(screen.getByText("Tell the app about your lights.")).toBeTruthy();
    await flush();
    expect(api.chains.start).not.toHaveBeenCalled();
    await press("Start");
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
  });

  it("it stays shut when every chain is set up or empty", async () => {
    await show([
      chain(1, "JARGB 1", "set_up", [part(1, "Fan 1", 120, 0, "fan")], 120),
      chain(2, "JARGB 2", "empty"),
    ]);

    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(api.chains.start).not.toHaveBeenCalled();
  });

  it("a header sized 8 with no chain record is still offered", async () => {
    await show([chain(1, "JARGB 1", "not_set_up", [], 8)]);

    // A saved size is not a record: the header is offered so its devices can be checked (#470).
    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    expect(screen.getByText(/MSI MYSTIC LIGHT · JARGB 1/)).toBeTruthy();
    expect(screen.getByText("Tell the app about your lights.")).toBeTruthy();
    await press("Start");
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
  });

  it("Luke's state offers the sized header with no record and none of the empty ones", async () => {
    await show([
      chain(0, "JAF", "empty"),
      chain(1, "JARGB 1", "not_set_up", [], 8),
      chain(2, "JARGB 2", "empty"),
      chain(3, "JARGB 3", "empty"),
    ]);

    // Zones 0, 2 and 3 have empty records, so they are answered and never offered again. Zone 1
    // has a size from a saved re-apply but no record, so it is the one offered (#470).
    expect(screen.getByText(/MSI MYSTIC LIGHT · JARGB 1/)).toBeTruthy();
    expect(screen.queryByText(/MSI MYSTIC LIGHT · JAF/)).toBeNull();
    expect(screen.queryByText(/MSI MYSTIC LIGHT · JARGB 2/)).toBeNull();
    expect(screen.queryByText(/MSI MYSTIC LIGHT · JARGB 3/)).toBeNull();
    await press("Start");
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
  });

  it("Later closes it and the header is not offered again, in this launch or the next", async () => {
    const first = await show([chain(1, "JARGB 1", "not_set_up")]);

    await press("Start");
    await press("Later");
    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(chainLaterThisLaunch()).toBe(true);
    first.unmount();

    // The same launch, a reload: the list is read again and the takeover stays shut.
    await show([chain(1, "JARGB 1", "not_set_up")]);
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    cleanup();

    // The next app start: a new launch, and the header is still put off (#380).
    window.sessionStorage.clear();
    await show([chain(1, "JARGB 1", "not_set_up")]);
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
  });

  it("headers are offered one after another", async () => {
    const second = chain(2, "JARGB 2", "not_set_up");
    await show([chain(1, "JARGB 1", "not_set_up"), second]);
    expect(screen.getByText(/MSI MYSTIC LIGHT · JARGB 1/)).toBeTruthy();

    // The first header is answered: the list says it is set up, and the takeover moves on.
    api.chains.list.mockResolvedValue({
      chains: [chain(1, "JARGB 1", "set_up", [part(1, "Fan 1", 120, 0, "fan")], 120), second],
    });
    await finish();
    await press("Done");
    await flush();

    expect(screen.getByText(/MSI MYSTIC LIGHT · JARGB 2/)).toBeTruthy();
    await press("Start");
    expect(api.chains.start).toHaveBeenLastCalledWith(DEVICE, 2);
  });

  it("the introduction is drawn once per visit and the next header gets the short card", async () => {
    const second = chain(2, "JARGB 2", "not_set_up");
    await show([chain(1, "JARGB 1", "not_set_up"), second]);
    expect(screen.getByText("Tell the app about your lights.")).toBeTruthy();

    api.chains.list.mockResolvedValue({
      chains: [chain(1, "JARGB 1", "set_up", [part(1, "Fan 1", 120, 0, "fan")], 120), second],
    });
    await finish();
    await press("Done");
    await flush();

    expect(screen.getByText("Next: JARGB 2.")).toBeTruthy();
    expect(screen.getByText("Is anything plugged into it?")).toBeTruthy();
    expect(screen.queryByText("Tell the app about your lights.")).toBeNull();
    // The card has the same three keys, and no session is open until Start.
    expect(screen.getByRole("button", { name: "Start" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Nothing is plugged in here" })).toBeTruthy();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
  });

  it("a header marked empty on the introduction opens the next card with the notice above it", async () => {
    const second = chain(2, "JARGB 2", "not_set_up");
    await show([chain(1, "JARGB 1", "not_set_up"), second]);

    api.chains.list.mockResolvedValue({ chains: [chain(1, "JARGB 1", "empty"), second] });
    await press("Nothing is plugged in here");
    await flush();

    expect(api.chains.empty).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(screen.getByText("Next: JARGB 2.")).toBeTruthy();
    expect(
      screen.getByText("JARGB 1 is marked empty. Set it up again from Devices, then add its lights to your scenes."),
    ).toBeTruthy();
  });

  it("an empty header is not offered again", async () => {
    const second = chain(2, "JARGB 2", "not_set_up");
    await show([chain(1, "JARGB 1", "empty"), second]);

    // Only the header with something to ask about is offered.
    expect(screen.queryByText(/JARGB 1/)).toBeNull();
    expect(screen.getByText(/MSI MYSTIC LIGHT · JARGB 2/)).toBeTruthy();

    api.chains.list.mockResolvedValue({
      chains: [chain(1, "JARGB 1", "empty"), chain(2, "JARGB 2", "set_up", [part(1, "Fan 1", 120, 0, "fan")], 120)],
    });
    await finish();
    await press("Done");
    await flush();

    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 2);
  });

  it("leaving the takeover by Escape cancels the session", async () => {
    await show([chain(1, "JARGB 1", "not_set_up")]);
    await press("Start");
    expect(api.chains.start).toHaveBeenCalledTimes(1);

    await act(async () => {
      fireEvent.keyDown(window, { key: "Escape" });
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(screen.queryByRole("region", SETUP)).toBeNull();
  });

  it("the standing panel opens the takeover and names no walker", async () => {
    await show([chain(1, "JARGB 1", "not_set_up")]);
    await press("Later");
    expect(screen.queryByRole("region", SETUP)).toBeNull();

    // The panel is the way back in after Later: it is on every page, and it offers the takeover again.
    nav.path = "/devices/";
    patchSnap({ status: { ...STATUS, show_zone_panel: true } as Status, devices: [] });
    render(<StandingNotices />);
    const title = screen.getByText("Set up your ARGB headers");
    const panel = title.closest(".banner") as HTMLElement;
    expect(panel.textContent).not.toMatch(/walker|identify walker|Size your ARGB/i);
    expect(panel.querySelector("a")).toBeNull();

    await press("Set up");

    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    await press("Start");
    expect(api.chains.start).toHaveBeenCalledTimes(1);
  });

  it("a page change the owner cancels leaves no request behind", async () => {
    await show([chain(1, "JARGB 1", "not_set_up")]);
    await press("Later");
    patchSnap({ status: { ...STATUS, show_zone_panel: true } as Status, devices: [] });
    render(<StandingNotices />);

    // An editor with unsaved work holds the page change, and the owner keeps editing.
    setNavigationGuard(() => false);
    try {
      await press("Set up");
      expect(screen.queryByRole("region", SETUP)).toBeNull();
      cancelNavigation();
      await flush();
      expect(hasPendingNavigation()).toBe(false);
      expect(screen.queryByRole("region", SETUP)).toBeNull();
    } finally {
      setNavigationGuard(null);
    }
  });
});
