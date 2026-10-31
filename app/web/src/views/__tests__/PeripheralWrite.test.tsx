import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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

describe("a reload that is still out when another device is picked", () => {
  /** Two devices; a write to the first starts a reload of it that stays out until `settle` runs. */
  async function reloadOfFirstIsOut() {
    const first = info("x6:first", "First Mouse");
    const second = info("x6:second", "Second Mouse");
    api.peripherals.mockResolvedValue({ peripherals: [first, second] });
    api.putPeripheralAccess.mockResolvedValue({ reads: true, writes: false });
    let firstReads = 0;
    let settle!: { resolve: (d: PeripheralDetail) => void; reject: (e: unknown) => void };
    api.peripheral.mockImplementation((id: string) => {
      if (id === "x6:second") return Promise.resolve(detail("x6:second", "Second Mouse"));
      firstReads += 1;
      // The first read of the first device answers; the reload after the write does not; a read
      // after that answers again.
      if (firstReads !== 2) return Promise.resolve(detail("x6:first", "First Mouse"));
      return new Promise<PeripheralDetail>((resolve, reject) => {
        settle = { resolve, reject };
      });
    });
    render(
      <ToastProvider>
        <PeripheralsView />
      </ToastProvider>,
    );
    const reads = await screen.findByRole("switch", { name: "Read settings from this device" });
    // The write lands, and the reload of the first device starts.
    fireEvent.click(reads);
    await waitFor(() => expect(firstReads).toBe(2));
    // The second device is picked while that reload is out, and its read answers at once.
    fireEvent.click(screen.getByText("Second Mouse"));
    await screen.findByRole("heading", { level: 2, name: "Second Mouse" });
    return { settle: () => settle };
  }

  /** The frames on the page that are waiting or refreshing. */
  const busy = () => document.querySelectorAll('.load-frame[aria-busy="true"]');

  it("does not leave the second device refreshing for good", async () => {
    await reloadOfFirstIsOut();
    // The second device is on screen and nothing reads it: no frame is busy.
    await waitFor(() => expect(busy()).toHaveLength(0));
  });

  it("does not leave the first device refreshing when it is picked again", async () => {
    await reloadOfFirstIsOut();
    // Back to the first device: its read answers, and the reload that was dropped must not
    // keep the page marked as refreshing.
    fireEvent.click(screen.getByText("First Mouse"));
    await screen.findByRole("heading", { level: 2, name: "First Mouse" });
    await waitFor(() => expect(busy()).toHaveLength(0));
  });

  it("drops a late failure of the first device's reload", async () => {
    const { settle } = await reloadOfFirstIsOut();
    await act(async () => {
      settle().reject(new Error("the first mouse did not answer"));
      await Promise.resolve();
      await Promise.resolve();
    });
    // The error belongs to the first device, which is not on screen.
    expect(screen.queryByText("The device could not be read")).toBeNull();
    expect(screen.queryByText("the first mouse did not answer")).toBeNull();
    expect(screen.getByRole("heading", { level: 2, name: "Second Mouse" })).toBeTruthy();
    expect(busy()).toHaveLength(0);
  });

  it("drops a late answer of the first device's reload", async () => {
    const { settle } = await reloadOfFirstIsOut();
    await act(async () => {
      settle().resolve(detail("x6:first", "First Mouse"));
      await Promise.resolve();
      await Promise.resolve();
    });
    // The second device stays on screen, read and not busy.
    expect(screen.getByRole("heading", { level: 2, name: "Second Mouse" })).toBeTruthy();
    expect(busy()).toHaveLength(0);
  });
});
