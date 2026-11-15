// #380: the chain setup opened by itself every time the window was opened from the tray.
//
// The takeover is the part that offers a header by itself. What is checked here: a header is
// offered at most once, ever; "Later", the close key and Escape are remembered per header, so a
// window opened from the tray (a new webview, and so a new launch) does not offer it again; a
// header that already has a size is never offered by itself; and a second header still gets its
// one offer.
//
// The daemon is the mocked chain routes, answered the way the other chain tests answer them, so
// what is checked is the takeover against the store it reads and the flow it draws.

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
import type { ChainSetupState } from "@/proto/ChainSetupState";
import type { ChainView } from "@/proto/ChainView";
import type { Status } from "@/proto/Status";
import { ChainTakeover } from "@/views/chain/ChainTakeover";
import { chainKey, offeredChains } from "@/views/chain/offered";
import { requestChainSetup } from "@/views/chain/request";

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
});

const DEVICE = "openrgb:A1B2C3D4E5F6";
const SESSION = "session-1";
const SETUP = { name: "Chain setup" };

const STATUS = {
  onboarding_complete: true,
  show_zone_panel: false,
  conflicts: [],
  active_scene: "gaming",
  openrgb: "connected",
} as unknown as Status;

/**
 * One resizable header as the daemon lists it. `leds` is the length it has now: 0 means the app
 * does not know how long the header is, which is what the flow's first step is for.
 */
function chain(zone: number, name: string, state: ChainSetupState, leds: number): ChainView {
  return {
    device_id: DEVICE,
    device_name: "MSI MYSTIC LIGHT",
    zone_index: zone,
    zone_name: name,
    state,
    leds,
    min_leds: 0,
    max_leds: 300,
    parts: [],
  };
}

/** Mounts the takeover with the list the daemon answers, and lets the reads settle. */
async function show(chains: ChainView[]) {
  api.chains.list.mockResolvedValue({ chains });
  patchSnap({ status: STATUS, chains });
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
 * The window comes up from the tray: the editor window is destroyed on close and a new webview
 * is made, so it has no session storage of its own and starts a new launch.
 */
function trayOpen(): void {
  window.sessionStorage.clear();
}

describe("the header setup the app offers by itself", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    for (const fn of Object.values(api.chains)) fn.mockReset();
    api.lights.list.mockReset();
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 0, raised_to: 300 });
    api.chains.light.mockResolvedValue({ ok: true });
    api.chains.cancel.mockResolvedValue({ ok: true });
    api.chains.empty.mockResolvedValue({ ok: true });
    api.chains.commit.mockResolvedValue({ lights: [], final_leds: 8 });
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

  it("a header the app has not sized is offered by itself", async () => {
    await show([chain(1, "JARGB 1", "not_set_up", 0)]);

    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    await flush();
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
  });

  it("a header that already has a size is never offered by itself", async () => {
    await show([chain(1, "JARGB 1", "not_set_up", 8)]);

    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(api.chains.start).not.toHaveBeenCalled();
  });

  it("Later is remembered when the window is opened again", async () => {
    const first = await show([chain(1, "JARGB 1", "not_set_up", 0)]);
    await press("Later");
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    first.unmount();

    // The window comes up from the tray: a new launch, and the header is still put off.
    trayOpen();
    await show([chain(1, "JARGB 1", "not_set_up", 0)]);

    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    expect(offeredChains().has(chainKey(DEVICE, 1))).toBe(true);
  });

  it("Escape is remembered when the window is opened again", async () => {
    const first = await show([chain(1, "JARGB 1", "not_set_up", 0)]);
    await flush();
    expect(api.chains.start).toHaveBeenCalledTimes(1);

    await act(async () => {
      fireEvent.keyDown(window, { key: "Escape" });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    first.unmount();

    trayOpen();
    await show([chain(1, "JARGB 1", "not_set_up", 0)]);

    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
  });

  it("the close key leaves the header put off as well", async () => {
    const first = await show([chain(1, "JARGB 1", "not_set_up", 0)]);
    await flush();

    // The flow ends: the daemon says the header is set up, and the close key ends the visit.
    api.chains.list.mockResolvedValue({ chains: [chain(1, "JARGB 1", "set_up", 8)] });
    await press("Next");
    await press("Next");
    await press("Yes");
    await press("Close");
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    first.unmount();

    trayOpen();
    await show([chain(1, "JARGB 1", "not_set_up", 0)]);

    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
  });

  it("a second header still gets its one offer", async () => {
    const first = await show([chain(1, "JARGB 1", "not_set_up", 0), chain(2, "JARGB 2", "not_set_up", 0)]);
    expect(screen.getByText(/JARGB 1/)).toBeTruthy();
    await press("Later");
    first.unmount();

    // A new launch: the first header is put off for good, the second one has its own offer.
    trayOpen();
    await show([chain(1, "JARGB 1", "not_set_up", 0), chain(2, "JARGB 2", "not_set_up", 0)]);

    expect(screen.queryByText(/JARGB 1/)).toBeNull();
    expect(screen.getByText(/JARGB 2/)).toBeTruthy();
    await flush();
    expect(api.chains.start).toHaveBeenLastCalledWith(DEVICE, 2);
  });

  // The request counter is the module's own state, so this is the last test in this file: a
  // request makes every later visit one the owner asked for.
  it("the owner can ask for the setup of a header the app would not offer", async () => {
    await show([chain(1, "JARGB 1", "not_set_up", 8)]);
    expect(screen.queryByRole("region", SETUP)).toBeNull();

    requestChainSetup();
    await flush();

    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    await flush();
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
  });
});
