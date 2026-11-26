import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

// #572: a pad keeps its id when it moves from the receiver to Bluetooth. The kept read is keyed
// by connection, so the old connection's sections and enabled Change never show on the new one.

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

async function tick(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

const WHY =
  "Over Bluetooth the pad sends its buttons, sticks, triggers and battery only. Its profiles, button map and stick ranges can be read and changed through the 2.4G receiver or a USB cable.";

const RECEIVER = {
  name: "8BitDo Ultimate 2 Wireless (XInput)",
  location: "hid:2dc8:310b",
};
const BLUETOOTH = {
  name: "8BitDo Ultimate 2 Wireless (Bluetooth)",
  location: "ble:2dc8:6012",
};

function row(id: string, info: typeof RECEIVER) {
  return {
    id,
    ...info,
    vendor: "8BitDo",
    kind: "game_controller",
    adapter: "eightbitdo-ultimate-2",
    serial: "",
    connected: true,
  };
}

function detail(id: string, info: typeof RECEIVER, withSettings: boolean): PeripheralDetail {
  return {
    info: {
      id,
      ...info,
      vendor: "8BitDo",
      kind: "game_controller",
      adapter: "eightbitdo-ultimate-2",
      serial: "",
      connected: true,
    },
    schema: {
      device: id,
      kind: "game_controller",
      sections: withSettings
        ? [
            {
              id: "profiles",
              title: "Profiles",
              properties: [
                {
                  key: "active-profile",
                  label: "Active profile",
                  description: "",
                  kind: { type: "choice", options: ["1"], default: "1" },
                },
              ],
            },
          ]
        : [],
      read_only: [],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values: withSettings ? { "active-profile": { type: "choice", value: "1" } } : {},
    access: { reads: true, writes: true },
    problem: withSettings ? null : WHY,
    read_at_ms: withSettings ? 1_700_000_000_000 : null,
  } as unknown as PeripheralDetail;
}

const toggle = (name: string) => screen.getByRole("switch", { name }) as HTMLButtonElement;

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

describe("the kept read of a pad that changes connection", () => {
  it("shows no receiver sections or enabled Change until the Bluetooth answer lands", async () => {
    vi.useFakeTimers();
    const id = "pad:connection-cache";

    // First visit: a receiver read is kept in the page's cache.
    api.peripherals.mockResolvedValue({ peripherals: [row(id, RECEIVER)] });
    api.peripheral.mockResolvedValue(detail(id, RECEIVER, true));
    const first = render(<PeripheralsView />);
    await tick(50);
    expect(screen.queryByText("Profiles")).not.toBeNull();
    expect(toggle("Change settings on the device").disabled).toBe(false);
    first.unmount();

    // The pad is now on Bluetooth. The detail GET is held back: the list already says which
    // connection it is on, so the page must not borrow the receiver's sections while it waits.
    let answer: (d: PeripheralDetail) => void = () => {};
    api.peripherals.mockResolvedValue({ peripherals: [row(id, BLUETOOTH)] });
    api.peripheral.mockImplementation(() => new Promise<PeripheralDetail>((resolve) => { answer = resolve; }));
    render(<PeripheralsView />);
    await tick(50);
    expect(screen.queryByText("Profiles")).toBeNull();
    const change = screen.queryByRole("switch", { name: "Change settings on the device" }) as HTMLButtonElement | null;
    expect(change === null || change.disabled).toBe(true);

    // The fresh answer has no settings: the page says so and keeps Change off.
    await act(async () => { answer(detail(id, BLUETOOTH, false)); });
    expect(screen.queryByText(WHY)).not.toBeNull();
    expect(screen.queryByText("Profiles")).toBeNull();
    expect(toggle("Change settings on the device").disabled).toBe(true);
  });
});
