// #380: the chain setup opened by itself every time the window was opened from the tray.
//
// The takeover is the part that offers a header by itself. What is checked here: a header is
// offered at most once, ever; "Later", the close key and Escape are remembered per header, so a
// window opened from the tray (a new webview, and so a new launch) does not offer it again; a
// header that already has a size is never offered by itself; "Later" puts off every header the
// visit has not shown yet; and a request is spent by the visit that serves it, so the setup does
// not open by itself on a later visit.
//
// The daemon is the mocked chain routes, answered the way the other chain tests answer them, so
// what is checked is the takeover against the store it reads and the flow it draws.

import { StrictMode } from "react";
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
import { chainKey, markChainOffered, offeredChains } from "@/views/chain/offered";
import { requestChainSetup } from "@/views/chain/request";
import { chainLaterThisLaunch } from "@/views/editor/flow";

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
const SETUP = { name: "Light setup" };

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

/** Mounts the takeover in strict mode, where every effect is run twice. */
async function showStrict(chains: ChainView[]) {
  api.chains.list.mockResolvedValue({ chains });
  patchSnap({ status: STATUS, chains });
  const view = render(
    <StrictMode>
      <ChainTakeover />
    </StrictMode>,
  );
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

/** Walks the flow from the introduction to the save, and closes it with Done. */
async function finish(): Promise<void> {
  await press("Start");
  await press("Next");
  await press("Next");
  await press("Next");
  await press("Yes, save these devices");
  await press("Done");
}

/**
 * The window comes up from the tray: the editor window is destroyed on close and a new webview
 * is made, so it has no session storage of its own and starts a new launch.
 */
function trayOpen(): void {
  window.sessionStorage.clear();
}

const originalStorage = Object.getOwnPropertyDescriptor(window, "localStorage");

/** Puts a storage that cannot be read or written in the window's place, as a locked webview has. */
function lockStorage(): void {
  const refuse = () => {
    throw new Error("no storage");
  };
  Object.defineProperty(window, "localStorage", {
    value: {
      getItem: refuse,
      setItem: refuse,
      removeItem: refuse,
      clear: refuse,
      key: refuse,
      length: 0,
    },
    configurable: true,
    writable: true,
  });
}

/** The window's own storage again. */
function unlockStorage(): void {
  if (originalStorage) Object.defineProperty(window, "localStorage", originalStorage);
  else Reflect.deleteProperty(window, "localStorage");
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
    unlockStorage();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  // The record is kept for the life of the page, so this test comes first: the storage has to
  // fail from the very first read for the mark to be lost.
  it("the header is not offered again in the same window when the storage cannot be used", async () => {
    // A webview with its storage locked throws on every read and on every write.
    lockStorage();

    const first = await show([chain(9, "JARGB 9", "not_set_up", 0)]);
    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    first.unmount();

    // The same window: the mark is kept in memory, so the header is not offered again.
    await show([chain(9, "JARGB 9", "not_set_up", 0)]);
    expect(screen.queryByRole("region", SETUP)).toBeNull();
  });

  it("a header the app has not sized is offered by itself", async () => {
    await show([chain(1, "JARGB 1", "not_set_up", 0)]);

    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    await flush();
    // The introduction first: the session opens on Start.
    expect(api.chains.start).not.toHaveBeenCalled();
    await press("Start");
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
    // Later on the introduction opened no session, then or now.
    expect(api.chains.start).not.toHaveBeenCalled();
    expect(offeredChains().has(chainKey(DEVICE, 1))).toBe(true);
  });

  it("the header's own record keeps it shut once the launch that said Later is over", async () => {
    const first = await show([chain(1, "JARGB 1", "not_set_up", 0)]);
    await press("Later");
    first.unmount();

    // The window comes up from the tray: a new launch, and the launch's own flag is gone with it.
    trayOpen();
    expect(chainLaterThisLaunch()).toBe(false);

    // The header's own record is what keeps it shut.
    await show([chain(1, "JARGB 1", "not_set_up", 0)]);
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(offeredChains().has(chainKey(DEVICE, 1))).toBe(true);
  });

  it("Escape is remembered when the window is opened again", async () => {
    const first = await show([chain(1, "JARGB 1", "not_set_up", 0)]);
    await press("Start");
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
    await finish();
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    first.unmount();

    trayOpen();
    await show([chain(1, "JARGB 1", "not_set_up", 0)]);

    expect(screen.queryByRole("region", SETUP)).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
  });

  it("Later puts off every header the visit has not shown yet", async () => {
    const first = await show([chain(1, "JARGB 1", "not_set_up", 0), chain(2, "JARGB 2", "not_set_up", 0)]);
    expect(screen.getByText(/MSI MYSTIC LIGHT · JARGB 1/)).toBeTruthy();
    await press("Later");
    first.unmount();

    // A new launch: the first header is put off for good, and so is the second one, because the
    // question is the same for both of them.
    trayOpen();
    await show([chain(1, "JARGB 1", "not_set_up", 0), chain(2, "JARGB 2", "not_set_up", 0)]);

    expect(screen.queryByText(/JARGB 1/)).toBeNull();
    expect(screen.queryByText(/JARGB 2/)).toBeNull();
    expect(api.chains.start).not.toHaveBeenCalled();
  });

  it("a request is spent by the visit that serves it", async () => {
    // The owner asks for the headers from the standing panel.
    const first = await show([chain(1, "JARGB 1", "not_set_up", 0), chain(2, "JARGB 2", "not_set_up", 8)]);
    requestChainSetup();
    await flush();
    expect(screen.getByRole("region", SETUP)).toBeTruthy();

    // The header is answered.
    api.chains.list.mockResolvedValue({
      chains: [chain(1, "JARGB 1", "set_up", 8), chain(2, "JARGB 2", "not_set_up", 8)],
    });
    await finish();
    first.unmount();

    // The next visit: the request is spent, so the app asks for nothing. A header is offered by
    // itself only if it has no size and has never been offered, and this one has a size.
    await show([chain(1, "JARGB 1", "set_up", 8), chain(2, "JARGB 2", "not_set_up", 8)]);
    expect(screen.queryByRole("region", SETUP)).toBeNull();
  });

  it("a request does not bring back a header an earlier launch put off", async () => {
    // An earlier launch put the second header off for good.
    markChainOffered(chainKey(DEVICE, 2));

    const first = await show([chain(1, "JARGB 1", "not_set_up", 0), chain(2, "JARGB 2", "not_set_up", 0)]);
    requestChainSetup();
    await flush();
    expect(screen.getByRole("region", SETUP)).toBeTruthy();

    api.chains.list.mockResolvedValue({
      chains: [chain(1, "JARGB 1", "set_up", 8), chain(2, "JARGB 2", "not_set_up", 0)],
    });
    await finish();
    first.unmount();

    // The next visit: the request is spent, and the header an earlier launch put off is not
    // offered by itself again.
    await show([chain(1, "JARGB 1", "set_up", 8), chain(2, "JARGB 2", "not_set_up", 0)]);
    expect(screen.queryByRole("region", SETUP)).toBeNull();
  });

  it("strict mode's double effect does not take the request before it is shown", async () => {
    requestChainSetup();
    await showStrict([chain(1, "JARGB 1", "not_set_up", 8)]);

    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    expect(screen.getByText(/MSI MYSTIC LIGHT · JARGB 1/)).toBeTruthy();
  });

  it("strict mode's double effect does not serve the request twice", async () => {
    requestChainSetup();
    await showStrict([chain(1, "JARGB 1", "not_set_up", 8)]);

    // One setup on screen, not two.
    expect(screen.getAllByRole("region", SETUP)).toHaveLength(1);
    // The request is spent by the visit that served it: the header does not come back.
    await press("Later");
    expect(screen.queryByRole("region", SETUP)).toBeNull();
  });

  // The owner can ask for the setup of a header the app would not offer. The request is spent by
  // the visit that serves it, so the visit before this one asked for nothing.
  it("the owner can ask for the setup of a header the app would not offer", async () => {
    await show([chain(1, "JARGB 1", "not_set_up", 8)]);
    expect(screen.queryByRole("region", SETUP)).toBeNull();

    requestChainSetup();
    await flush();

    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    await press("Start");
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
  });
});
