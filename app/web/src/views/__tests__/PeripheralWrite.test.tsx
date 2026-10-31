import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { PeripheralsView } from "@/views/PeripheralsView";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { PeripheralInfo } from "@/proto/PeripheralInfo";

// The peripherals page: what it says when a change landed and only the refresh failed, and
// whose error it shows when two devices are read at once.

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

/** One device the adapters see. */
function info(id: string, name: string): PeripheralInfo {
  return {
    id,
    name,
    vendor: "Attack Shark",
    kind: "mouse",
    adapter: "attack-shark-x6",
    serial: "",
    location: "",
    connected: true,
  };
}

/** What the device is and what it holds. */
function detail(id: string, name: string): PeripheralDetail {
  return {
    info: info(id, name),
    schema: { device: id, kind: "mouse", sections: [], read_only: [], unreported: [] },
    values: {},
    access: { reads: false, writes: false },
    problem: null,
  };
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

describe("a change that landed", () => {
  it("says the change was saved when only the refresh failed", async () => {
    const mouse = info("x6:one", "Attack Shark X6");
    api.peripherals.mockResolvedValue({ peripherals: [mouse] });
    // The first read answers; the refresh after the change does not.
    api.peripheral
      .mockResolvedValueOnce(detail("x6:one", "Attack Shark X6"))
      .mockRejectedValueOnce(new Error("the page could not be refreshed"));
    api.putPeripheralAccess.mockResolvedValue({ reads: true, writes: false });

    render(
      <ToastProvider>
        <PeripheralsView />
      </ToastProvider>,
    );
    // The device is on screen and its settings are there.
    const reads = await screen.findByRole("switch", { name: "Read settings from this device" });

    // The access is turned on: the change lands on the device, and the refresh fails.
    fireEvent.click(reads);

    await screen.findByText("The device could not be read");
    // The change is on the device: the page must not say that nothing was changed.
    expect(screen.getByText("The change was saved, but the page could not be refreshed.")).toBeTruthy();
    expect(screen.queryByText(/Nothing was changed/)).toBeNull();
  });

  it("says nothing was changed when the change itself failed", async () => {
    const mouse = info("x6:one", "Attack Shark X6");
    api.peripherals.mockResolvedValue({ peripherals: [mouse] });
    api.peripheral.mockResolvedValue(detail("x6:one", "Attack Shark X6"));
    api.putPeripheralAccess.mockRejectedValue(new Error("the device did not take the change"));

    render(
      <ToastProvider>
        <PeripheralsView />
      </ToastProvider>,
    );
    const reads = await screen.findByRole("switch", { name: "Read settings from this device" });

    fireEvent.click(reads);

    await screen.findByText("the device did not take the change");
    // The change did not land, so the page says so.
    expect(screen.queryByText("The change was saved, but the page could not be refreshed.")).toBeNull();
  });
});

describe("two devices at once", () => {
  it("does not put the error of a slow device on the one that is on screen", async () => {
    const slow = info("x6:slow", "Slow Mouse");
    const quick = info("x6:quick", "Quick Mouse");
    api.peripherals.mockResolvedValue({ peripherals: [slow, quick] });
    let failSlow!: (e: unknown) => void;
    api.peripheral.mockImplementation((id: string) => {
      if (id === "x6:slow") {
        return new Promise((_resolve, reject) => {
          failSlow = reject;
        });
      }
      return Promise.resolve(detail("x6:quick", "Quick Mouse"));
    });

    render(
      <ToastProvider>
        <PeripheralsView />
      </ToastProvider>,
    );
    // The slow device is picked first (the first row of the list).
    await screen.findByText("Slow Mouse");
    // The other device is picked: its read answers at once.
    fireEvent.click(screen.getByText("Quick Mouse"));
    await screen.findByRole("heading", { level: 2, name: "Quick Mouse" });

    // The slow read fails now: it belongs to the device that is no longer on screen.
    await act(async () => {
      failSlow(new Error("the mouse did not answer"));
      await Promise.resolve();
      await Promise.resolve();
    });

    // The page still shows the device that was picked, with no error on it.
    expect(screen.queryByText("The device could not be read")).toBeNull();
    expect(screen.getAllByText("Quick Mouse").length).toBeGreaterThan(0);
  });
});
