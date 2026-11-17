// One resizable header on the Devices page: what state it is in, the one key that starts or redoes
// its setup, and every part's own controls once it is set up.
//
// The routes are the mocked lights and chain routes, so what is checked is the row against them:
// what it says of a header in each state, that the flow opens for that header only, that a part's
// name, kind and direction go to the lights route without a session, and that a part a re-setup
// adds reaches Scenes' own new-light notice.

import { act, cleanup, fireEvent, render, renderHook, screen, within } from "@testing-library/react";
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
    lights: { list: vi.fn(), patch: vi.fn() },
    getWhiteBalance: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

const { nav } = vi.hoisted(() => ({ nav: { path: "/devices/" } }));
vi.mock("next/navigation", () => ({
  usePathname: () => nav.path,
  useRouter: () => ({ push: () => {}, replace: () => {}, prefetch: () => {} }),
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

const { sceneRef } = vi.hoisted(() => ({ sceneRef: { current: null as unknown } }));
vi.mock("@/views/scenes/useScene", () => ({ useScene: () => sceneRef.current }));
vi.mock("@/views/scenes/useLightFeed", () => ({ useLightFeed: () => () => null }));

import { getSnap, patchSnap } from "@/lib/store";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { ChainSetupState } from "@/proto/ChainSetupState";
import type { ChainView } from "@/proto/ChainView";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import { SEEN_LIGHTS_KEY } from "@/views/scenes/newLights";
import { ScenesView } from "@/views/scenes/ScenesView";
import { ChainRow } from "@/views/chain/ChainRow";
import { chainKey, offeredChains } from "@/views/chain/offered";
import { DevicesView } from "@/views/DevicesView";
import { useTakeover } from "@/views/editor/takeover";
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
  globalThis.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
});

const DEVICE = "openrgb:A1B2C3D4E5F6";
const SESSION = "session-1";
const SETUP = { name: "Light setup" };

function zone(index: number, name: string, leds: number, resizable: boolean): ZoneInfo {
  return {
    index,
    name,
    kind: { type: "linear" },
    led_count: leds,
    min_leds: 0,
    max_leds: 300,
    resizable,
  };
}

/** The board the demo and the fakes both answer with: three resizable headers beside the JAF. */
const BOARD: DeviceInfo = {
  id: DEVICE,
  name: "MSI MYSTIC LIGHT",
  vendor: "MSI",
  kind: "motherboard",
  adapter: "openrgb",
  adapter_index: 3,
  serial: "A1B2C3D4E5F6",
  location: "",
  connected: true,
  zones: [zone(0, "JAF", 0, true), zone(1, "JARGB 1", 120, true), zone(2, "JARGB 2", 96, true)],
};

/** One part of a set-up chain, by position (1-based), at `start` along its zone. */
function part(
  z: number,
  position: number,
  name: string,
  leds: number,
  start: number,
  kind: ChainPartKind,
): LightView {
  return {
    light: {
      id: `${DEVICE}#z${z}p${position}`,
      device_id: DEVICE,
      zone_index: z,
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

function chain(z: number, name: string, state: ChainSetupState, parts: LightView[] = [], leds = 120): ChainView {
  return {
    device_id: DEVICE,
    device_name: "MSI MYSTIC LIGHT",
    zone_index: z,
    zone_name: name,
    state,
    leds,
    min_leds: 0,
    max_leds: 300,
    parts,
  };
}

const FAN_ONE = part(2, 1, "Fan 1", 14, 0, "fan");
const FAN_TWO = part(2, 2, "Fan 2", 82, 14, "strip");

function rowOf(zoneName: string): HTMLElement {
  const row = screen
    .getAllByTestId("chain-row")
    .find((el) => el.getAttribute("aria-label") === `MSI MYSTIC LIGHT ${zoneName}`);
  if (!row) throw new Error(`no chain row for ${zoneName}`);
  return row;
}

/** Lets pending answers land and moves the clock by `ms`. */
async function flush(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** Mounts the Devices page with the board and the chains the daemon lists. */
async function showDevices(chains: ChainView[]): Promise<void> {
  api.chains.list.mockResolvedValue({ chains });
  patchSnap({
    devices: [BOARD],
    devicesTotal: 1,
    devicesError: null,
    lights: [],
    lightsError: null,
    chains,
    chainsError: null,
    status: null,
    statusError: null,
  });
  render(<DevicesView />);
  await flush();
  expect(screen.getAllByTestId("device-card").length).toBe(1);
}

describe("a header's row on Devices", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    for (const fn of Object.values(api.chains)) fn.mockReset();
    for (const fn of Object.values(api.lights)) fn.mockReset();
    api.getWhiteBalance.mockReset();
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 120, raised_to: 300 });
    api.chains.light.mockResolvedValue({ ok: true });
    api.chains.cancel.mockResolvedValue({ ok: true });
    api.chains.empty.mockResolvedValue({ ok: true });
    api.chains.commit.mockResolvedValue({ lights: [], final_leds: 120 });
    api.chains.list.mockResolvedValue({ chains: [] });
    api.lights.list.mockResolvedValue({ lights: [] });
    api.lights.patch.mockImplementation(async (id: string, body: Record<string, unknown>) => ({
      light: {
        id,
        device_id: DEVICE,
        zone_index: 2,
        arrangement: body.kind === "fan" ? "ring" : "strip",
        reversed: false,
      },
      led_count: 14,
      connected: true,
      label: "Fan 1",
    }));
    api.getWhiteBalance.mockResolvedValue({ devices: {} });
    window.localStorage.clear();
    patchSnap({ chains: null, lights: null });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("shows not set up, empty and its parts", async () => {
    await showDevices([
      chain(0, "JAF", "not_set_up", [], 0),
      chain(1, "JARGB 1", "empty"),
      chain(2, "JARGB 2", "set_up", [FAN_ONE, FAN_TWO], 96),
    ]);

    const rows = screen.getAllByTestId("chain-row");
    expect(rows).toHaveLength(3);
    const [jaf, jar, jarTwo] = rows as [HTMLElement, HTMLElement, HTMLElement];
    expect(jaf.textContent).toContain("Not set up");
    expect(jar.textContent).toContain("Empty");
    expect(jarTwo.textContent).toContain("2 devices");
    expect(within(jarTwo).getByText("Fan 1")).toBeTruthy();
    expect(within(jarTwo).getByText("Fan 2")).toBeTruthy();
    expect(within(jaf).getByRole("button", { name: /^Set up/ })).toBeTruthy();
    expect(within(jar).getByRole("button", { name: /^Set up/ })).toBeTruthy();
    expect(within(jarTwo).getByRole("button", { name: /^Set up again/ })).toBeTruthy();
  });

  it("a disconnected board's header row reads disconnected, not a hard-coded connected", async () => {
    // The daemon lists chains for devices that are not connected, so the row must say so from
    // DeviceInfo.connected instead of appending a literal " · connected" to the sub-line.
    const offline = { ...BOARD, connected: false };
    const chains = [chain(0, "JAF", "not_set_up", [], 0)];
    api.chains.list.mockResolvedValue({ chains });
    patchSnap({
      devices: [offline],
      devicesTotal: 1,
      devicesError: null,
      lights: [],
      lightsError: null,
      chains,
      chainsError: null,
      status: null,
      statusError: null,
    });
    render(<DevicesView />);
    await flush();
    const row = rowOf("JAF");
    expect(row.textContent).toContain("disconnected");
    expect(row.textContent).not.toContain(" · connected");
  });

  it("Set up again starts the flow for that header", async () => {
    await showDevices([chain(2, "JARGB 2", "set_up", [FAN_ONE, FAN_TWO], 96)]);

    await act(async () => {
      fireEvent.click(within(rowOf("JARGB 2")).getByRole("button", { name: /^Set up again/ }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await flush();

    // A flow opened from the row skips the introduction: the session opens at once.
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 2);
    const setup = screen.getByRole("region", SETUP);
    expect(within(setup).getByText(/JARGB 2/)).toBeTruthy();
    expect(screen.queryByText("Tell the app about your lights.")).toBeNull();
    // It stands over the page (the takeover's overlay), not inside the row: the rows below stay put.
    expect(rowOf("JARGB 2").contains(setup)).toBe(false);
  });

  it("the overlay holds the window while it is open and gives it back", async () => {
    await showDevices([chain(1, "JARGB 1", "not_set_up", [], 0)]);
    const held = renderHook(() => useTakeover());
    expect(held.result.current).toBe(false);

    await act(async () => {
      fireEvent.click(within(rowOf("JARGB 1")).getByRole("button", { name: /^Set up/ }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await flush();
    expect(held.result.current).toBe(true);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Later" }));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(held.result.current).toBe(false);
  });

  it("Escape on the overlay is Later: it closes, ends the session and writes no Later flag", async () => {
    await showDevices([chain(1, "JARGB 1", "not_set_up", [], 0)]);

    await act(async () => {
      fireEvent.click(within(rowOf("JARGB 1")).getByRole("button", { name: /^Set up/ }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await flush();
    expect(screen.getByRole("region", SETUP)).toBeTruthy();

    await act(async () => {
      fireEvent.keyDown(window, { key: "Escape" });
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.queryByRole("region", SETUP)).toBeNull();
    // Later: the session ends, so the header goes back to the length it had.
    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    // The row writes no "Later" for the launch: a row's close is not the takeover's.
    expect(chainLaterThisLaunch()).toBe(false);

    // And the row's key opens the header again whenever the owner asks.
    await act(async () => {
      fireEvent.click(within(rowOf("JARGB 1")).getByRole("button", { name: /^Set up/ }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await flush();
    expect(screen.getByRole("region", SETUP)).toBeTruthy();
  });

  it("Escape inside the device count puts the value back and leaves the overlay open", async () => {
    await showDevices([chain(1, "JARGB 1", "not_set_up", [], 0)]);

    await act(async () => {
      fireEvent.click(within(rowOf("JARGB 1")).getByRole("button", { name: /^Set up/ }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await flush();
    // Counting the orange lights, the slide, then the devices step's count (review finding 1).
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    await flush(700);

    const count = screen.getByRole("textbox", { name: "Devices on JARGB 1" }) as HTMLInputElement;
    fireEvent.focus(count);
    fireEvent.change(count, { target: { value: "4" } });
    expect(count.value).toBe("4");

    await act(async () => {
      fireEvent.keyDown(count, { key: "Escape" });
      await vi.advanceTimersByTimeAsync(0);
    });

    // Escape was the field's own: the overlay is still open, the typed value is gone and the
    // session was never cancelled.
    expect(screen.getByRole("region", SETUP)).toBeTruthy();
    expect(count.value).toBe("1");
    expect(api.chains.cancel).not.toHaveBeenCalled();
  });

  it("a header whose devices mirror each other reads as one light shown several times", async () => {
    const mirror = part(2, 1, "Same lights, shown 8 times", 8, 0, "other");
    mirror.light.copies = 8;
    await showDevices([chain(2, "JARGB 2", "set_up", [mirror], 8)]);

    const row = rowOf("JARGB 2");
    expect(row.textContent).toContain("8 LEDs · shown 8 times");
    expect(row.textContent).toContain("You cannot set one on its own.");
    expect(row.textContent).toContain("ARGB header");
    expect(within(row).getByRole("list", { name: "Devices on JARGB 2" })).toBeTruthy();
    expect(row.textContent).not.toMatch(/parts?/i);
  });

  it("an empty header says nothing is plugged into this ARGB header", async () => {
    await showDevices([chain(1, "JARGB 1", "empty")]);
    expect(rowOf("JARGB 1").textContent).toContain("Nothing is plugged into this ARGB header.");
  });

  it("a set up header lists every part with its own controls", async () => {
    await showDevices([chain(2, "JARGB 2", "set_up", [FAN_ONE, FAN_TWO], 96)]);

    const parts = within(rowOf("JARGB 2")).getAllByTestId("chain-part");
    expect(parts).toHaveLength(2);
    for (const [index, el] of parts.entries()) {
      const at = within(el);
      expect(at.getByRole("button", { name: `Rename ${index === 0 ? "Fan 1" : "Fan 2"}` })).toBeTruthy();
      expect(at.getByRole("radio", { name: "Fan" })).toBeTruthy();
      expect(at.getByRole("radio", { name: "Strip" })).toBeTruthy();
      expect(at.getByRole("radio", { name: "Other" })).toBeTruthy();
      expect(at.getByRole("switch", { name: /Runs the other way/ })).toBeTruthy();
      expect(at.queryByRole("radio", { name: "Ring" })).toBeNull();
    }
  });

  it("renaming a part calls the lights route without opening the flow", async () => {
    await showDevices([chain(2, "JARGB 2", "set_up", [FAN_ONE, FAN_TWO], 96)]);

    const first = within(rowOf("JARGB 2")).getAllByTestId("chain-part")[0] as HTMLElement;
    await act(async () => {
      fireEvent.click(within(first).getByRole("button", { name: "Rename Fan 1" }));
    });
    const field = within(first).getByRole("textbox", { name: "Name for Fan 1" }) as HTMLInputElement;
    await act(async () => {
      fireEvent.change(field, { target: { value: "Top intake" } });
      fireEvent.keyDown(field, { key: "Enter" });
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.lights.patch).toHaveBeenCalledWith(`${DEVICE}#z2p1`, { name: "Top intake" });
    expect(api.chains.start).not.toHaveBeenCalled();
    expect(screen.queryByRole("region", SETUP)).toBeNull();
  });

  it("a part with a kind shows Fan, Strip and Other and sends kind", async () => {
    await showDevices([chain(2, "JARGB 2", "set_up", [FAN_ONE, FAN_TWO], 96)]);

    const first = within(rowOf("JARGB 2")).getAllByTestId("chain-part")[0] as HTMLElement;
    await act(async () => {
      fireEvent.click(within(first).getByRole("radio", { name: "Strip" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.lights.patch).toHaveBeenCalledWith(`${DEVICE}#z2p1`, { kind: "strip" });
    expect(api.chains.start).not.toHaveBeenCalled();
  });

  it("a setup opened from the row counts as that header's one offer", async () => {
    await showDevices([chain(1, "JARGB 1", "not_set_up", [], 0)]);

    await act(async () => {
      fireEvent.click(within(rowOf("JARGB 1")).getByRole("button", { name: /^Set up/ }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await flush();
    expect(screen.getByRole("region", SETUP)).toBeTruthy();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Later" }));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.queryByRole("region", SETUP)).toBeNull();

    // The header was on screen: the app does not offer it by itself on the next visit to Scenes.
    expect(offeredChains().has(chainKey(DEVICE, 1))).toBe(true);
  });
});

describe("a part a re-setup adds", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    for (const fn of Object.values(api.chains)) fn.mockReset();
    for (const fn of Object.values(api.lights)) fn.mockReset();
    api.getWhiteBalance.mockReset();
    api.getWhiteBalance.mockResolvedValue({ devices: {} });
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 120, raised_to: 300 });
    api.chains.light.mockResolvedValue({ ok: true });
    api.chains.cancel.mockResolvedValue({ ok: true });
    api.chains.empty.mockResolvedValue({ ok: true });
    api.chains.list.mockResolvedValue({ chains: [] });
    api.lights.list.mockResolvedValue({ lights: [] });
    window.localStorage.clear();
    patchSnap({ chains: null, lights: null, status: null, statusError: null });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("appears in the new-light notice", async () => {
    const before = part(2, 1, "Fan 1", 96, 0, "fan");
    const after = [part(2, 1, "Fan 1", 48, 0, "fan"), part(2, 2, "Fan 2", 48, 48, "strip")];
    // The page has seen the parts from before the re-setup: only the new one may be announced.
    window.localStorage.setItem(SEEN_LIGHTS_KEY, JSON.stringify([before.light.id]));

    api.chains.commit.mockResolvedValue({ lights: after, final_leds: 96 });
    api.lights.list.mockResolvedValue({ lights: after });
    api.chains.list.mockResolvedValue({
      chains: [chain(2, "JARGB 2", "set_up", after, 96)],
    });

    const row = render(<ChainRow chain={chain(2, "JARGB 2", "set_up", [before], 96)} />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^Set up again/ }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await flush();
    // Count the orange lights (one), slide (the stored 96), then the devices (two).
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    const count = screen.getByRole("textbox", { name: "Devices on JARGB 2" });
    await act(async () => {
      fireEvent.change(count, { target: { value: "2" } });
      fireEvent.blur(count);
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Yes, save these devices" }));
      await vi.advanceTimersByTimeAsync(0);
    });
    await flush();

    // The commit is followed by a read of the lights, so the new part is in the store at once.
    expect(api.lights.list).toHaveBeenCalled();
    expect(getSnap().lights?.map((l) => l.light.id)).toEqual(after.map((l) => l.light.id));
    row.unmount();

    // Scenes holds the parts no group has, and its notice names the one it has not seen before.
    const group = {
      id: "g-case",
      name: "Case fans",
      members: [],
      plays: { type: "off" as const },
      mode: "flow_across" as const,
      brightness: 100,
    };
    const scene = { id: "gaming", name: "Gaming", rev: 1, groups: [group] } as Scene;
    sceneRef.current = {
      scene,
      scenes: [scene],
      unassigned: after,
      states: [{ id: group.id, failed: null }],
      edit: () => {},
      setProps: () => {},
      setBrightness: () => {},
      setActive: () => {},
      create: () => {},
      remove: () => {},
      renameScene: () => {},
      undo: () => {},
      redo: () => {},
      canUndo: false,
      canRedo: false,
      error: null,
    };
    render(<ScenesView />);
    await flush();

    const notice = screen.getByRole("region", { name: /Fan 2/ });
    expect(notice.textContent).not.toContain("Fan 1");
  });
});
