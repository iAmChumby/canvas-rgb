import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { PeripheralsView } from "@/views/PeripheralsView";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { PeripheralInfo } from "@/proto/PeripheralInfo";

// Issue #7: the Peripherals view reads the device once and asks again only on the Read again key.

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
});

/** The mouse, with reads on. */
function mouseDetail(): PeripheralDetail {
  const info: PeripheralInfo = {
    id: "x6:one",
    name: "Attack Shark X6",
    vendor: "Attack Shark",
    kind: "mouse",
    adapter: "attack-shark-x6",
    serial: "",
    location: "",
    connected: true,
  };
  return {
    info,
    schema: { device: "x6:one", kind: "mouse", sections: [], read_only: [], unreported: [] },
    values: {},
    access: { reads: true, writes: false },
    problem: null,
  };
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
  api.peripherals.mockResolvedValue({ peripherals: [mouseDetail().info] });
  api.peripheral.mockResolvedValue(mouseDetail());
});

function open() {
  return render(
    <ToastProvider>
      <PeripheralsView />
    </ToastProvider>,
  );
}

describe("the Read again key", () => {
  it("asks the daemon for a fresh read, and opening the view again does not", async () => {
    open();
    const key = await screen.findByRole("button", { name: "Read again" });
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(1));
    // Opening the view is a plain read: the daemon answers with what it kept.
    expect(api.peripheral).toHaveBeenLastCalledWith("x6:one");

    fireEvent.click(key);
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(2));
    expect(api.peripheral).toHaveBeenLastCalledWith("x6:one", { fresh: true });

    // The view is closed and opened again: plain reads only.
    cleanup();
    api.peripheral.mockClear();
    open();
    await screen.findByRole("button", { name: "Read again" });
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(1));
    for (const call of api.peripheral.mock.calls) {
      expect(call.length).toBe(1);
    }
  });

  it("is off while reads are off, because there is nothing to read", async () => {
    const off = mouseDetail();
    off.access = { reads: false, writes: false };
    api.peripheral.mockResolvedValue(off);
    open();
    const key = await screen.findByRole("button", { name: "Read again" });
    expect((key as HTMLButtonElement).disabled).toBe(true);
  });
});
