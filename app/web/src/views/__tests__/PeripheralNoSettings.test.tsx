import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView, UNREAD_POLL_MS } from "@/views/PeripheralsView";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

// A connection that carries no settings (a controller over Bluetooth): the page says so once, in
// the daemon's words, and offers no control that cannot work.

const { api } = vi.hoisted(() => ({
  api: {
    peripherals: vi.fn(),
    peripheral: vi.fn(),
    patchPeripheral: vi.fn(),
    putPeripheralAccess: vi.fn(),
    peripheralRead: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

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

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** Lets the page's promises settle and the clock run `ms`. */
async function tick(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

const WHY =
  "Over Bluetooth the pad sends its buttons, sticks, triggers and battery only. Its profiles, button map and stick ranges can be read and changed through the 2.4G receiver or a USB cable.";

/** A controller as the daemon answers it: with no sections on a connection that has no settings. */
function pad(id: string, over: Partial<PeripheralDetail> = {}, offered = false): PeripheralDetail {
  return {
    info: {
      id,
      name: offered ? "8BitDo Ultimate 2 Wireless (XInput)" : "8BitDo Ultimate 2 Wireless (Bluetooth)",
      vendor: "8BitDo",
      kind: "game_controller",
      adapter: "eightbitdo-ultimate-2",
      serial: "",
      location: offered ? "hid:2dc8:310b" : "ble:2dc8:6012",
      connected: true,
    },
    schema: {
      device: id,
      kind: "game_controller",
      sections: offered
        ? [
            {
              id: "profiles",
              title: "Profiles",
              properties: [
                { key: "active-profile", label: "Active profile", description: "", kind: { type: "choice", options: ["1"], default: "1" } },
              ],
            },
          ]
        : [],
      read_only: [],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values: {},
    access: { reads: true, writes: true },
    problem: offered ? "The device's settings have not been read yet." : WHY,
    ...over,
  };
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

function serve(id: string, detail: () => Promise<PeripheralDetail>) {
  api.peripherals.mockResolvedValue({ peripherals: [pad(id).info] });
  api.peripheral.mockImplementation(detail);
}

const toggle = (name: string) => screen.getByRole("switch", { name }) as HTMLButtonElement;

describe("a connection with no settings", () => {
  it("says why once, and promises no read", async () => {
    serve("pad:why", async () => pad("pad:why"));
    render(<PeripheralsView />);
    expect(await screen.findAllByText(WHY)).toHaveLength(1);
    expect(screen.queryByText(/Not read yet/, { selector: "p" })).toBeNull();
    expect(screen.queryByText(/read while you are away/)).toBeNull();
    expect(document.querySelector('[data-role="problem"]')).toBeNull();
    expect(document.querySelector('[data-role="read-at"]')?.getAttribute("data-empty")).toBe("true");
    expect(screen.queryByText("RGB rings")).toBeNull();
  });

  it("offers no Change and no Read again, and keeps Read for the battery", async () => {
    serve("pad:keys", async () => pad("pad:keys"));
    render(<PeripheralsView />);
    await screen.findByText(WHY);
    const change = toggle("Change settings on the device");
    expect(change.disabled).toBe(true);
    // The saved access says writes are on; the switch does not claim a change can be made.
    expect(change.getAttribute("aria-checked")).toBe("false");
    expect((screen.getByRole("button", { name: "Read again" }) as HTMLButtonElement).disabled).toBe(true);
    expect(toggle("Read settings from this device").disabled).toBe(false);
    expect(api.putPeripheralAccess).not.toHaveBeenCalled();
  });

  it("says the same with reads off, without offering to read the settings", async () => {
    serve("pad:off", async () => pad("pad:off", { access: { reads: false, writes: false } }));
    render(<PeripheralsView />);
    await screen.findByText(WHY);
    expect(screen.queryByText(/settings are read only when you turn reading on/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Turn on reading" })).toBeNull();
  });

  it("notices the controller moving to a connection that has settings", async () => {
    vi.useFakeTimers();
    const id = "pad:moves";
    let answer = pad(id);
    serve(id, async () => answer);
    render(<PeripheralsView />);
    await tick(50);
    expect(screen.queryByText(WHY)).not.toBeNull();
    answer = pad(id, {}, true);
    await tick(UNREAD_POLL_MS);
    expect(screen.queryByText(WHY)).toBeNull();
    expect(toggle("Change settings on the device").disabled).toBe(false);
    for (const call of api.peripheral.mock.calls) expect(call).toEqual([id]);
  });
});
