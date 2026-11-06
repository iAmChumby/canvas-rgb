import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

// Issue #156 (Peripherals): `saveError` is one slot for the whole view, so a failed write
// on device A is cleared when device B's read lands, and switching back to A loses it.
// And a late read calls `setStaged({})`, dropping edits the user made after that read
// started. The save error is per device key; a read started before the latest edit leaves
// the staged edits alone.

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

afterEach(cleanup);

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

/** One mouse with a switch in a section of its own. */
function mouse(id: string, name: string): PeripheralDetail {
  return {
    info: {
      id,
      name,
      vendor: "Attack Shark",
      kind: "mouse",
      adapter: "attack-shark-x6",
      serial: "",
      location: "",
      connected: true,
    },
    schema: {
      device: id,
      kind: "mouse",
      sections: [
        {
          id: "lighting",
          title: "Lighting",
          properties: [
            { key: "led-on", label: "LED on", kind: { type: "bool", default: false } },
            { key: "breathe", label: "Breathe", kind: { type: "bool", default: false } },
          ],
        },
      ],
      read_only: [],
      unreported: [],
    },
    values: { "led-on": { type: "bool", value: false }, breathe: { type: "bool", value: false } },
    access: { reads: true, writes: true },
    problem: null,
  };
}

/** The page, with the strip it draws its actions into. */
function page() {
  return render(
    <StripSlotsProvider>
      <header data-testid="strip">
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
      </header>
      <PeripheralsView />
    </StripSlotsProvider>,
  );
}

describe("a failed write on one device", () => {
  it("shows only on that device, and survives the other device's read", async () => {
    const a = mouse("x6:a", "A Mouse");
    const b = mouse("x6:b", "B Mouse");
    api.peripherals.mockResolvedValue({ peripherals: [a.info, b.info] });
    api.peripheral.mockImplementation((id: string) => Promise.resolve(id === "x6:a" ? a : b));
    api.patchPeripheral.mockRejectedValue(new Error("A write failed"));
    page();

    // Device A is picked first; the user changes a setting and sends it: it fails.
    const led = await screen.findByRole("switch", { name: "LED on" });
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(1));
    await act(async () => {
      await Promise.resolve();
    });
    fireEvent.click(led);
    fireEvent.click(await screen.findByRole("button", { name: "Apply" }));
    await waitFor(() => expect(api.patchPeripheral).toHaveBeenCalledTimes(1));
    await screen.findByText("A write failed");

    // Device B is picked: its read lands. B shows no error.
    fireEvent.click(screen.getByText("B Mouse"));
    await screen.findByRole("heading", { level: 2, name: "B Mouse" });
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(2));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText("A write failed")).toBeNull();

    // Back to device A: its read lands, and its own error is still there.
    fireEvent.click(screen.getByText("A Mouse"));
    await screen.findByRole("heading", { level: 2, name: "A Mouse" });
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(3));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText("A write failed")).toBeTruthy();
  });
});

describe("a read that lands after an edit", () => {
  it("keeps the edit the user made while the read was out", async () => {
    const device = mouse("x6:edit", "Edit Mouse");
    api.peripherals.mockResolvedValue({ peripherals: [device.info] });
    api.peripheral.mockResolvedValue(device);
    const first = page();
    // The first visit reads the device and leaves its values in the cache.
    await screen.findByRole("switch", { name: "LED on" });
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(1));
    await act(async () => {
      await Promise.resolve();
    });
    first.unmount();

    // The second visit: the cached values are on screen and the fresh read is out.
    let land!: (d: PeripheralDetail) => void;
    api.peripheral.mockImplementationOnce(
      () =>
        new Promise<PeripheralDetail>((r) => {
          land = r;
        }),
    );
    page();
    const led = await screen.findByRole("switch", { name: "LED on" });
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(2));

    // The user edits while the fresh read is out: the change is staged, Apply appears.
    fireEvent.click(led);
    await screen.findByRole("button", { name: "Apply" });

    // The fresh read lands now.
    await act(async () => {
      land(device);
      await Promise.resolve();
      await Promise.resolve();
    });

    // The edit is still staged: Apply is still there, counting the change.
    expect(screen.queryByRole("button", { name: "Apply" })).toBeTruthy();
    expect(screen.queryByText(/1 change not sent/)).toBeTruthy();
  });
});

/** Lets every pending promise and the renders they cause run. */
async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("an edit on one device", () => {
  it("is never shown on, or sent to, another device", async () => {
    const a = mouse("x6:a", "A Mouse");
    const b = mouse("x6:b", "B Mouse");
    api.peripherals.mockResolvedValue({ peripherals: [a.info, b.info] });
    api.peripheral.mockImplementation((id: string) => Promise.resolve(id === "x6:a" ? a : b));
    api.patchPeripheral.mockResolvedValue(undefined);
    page();
    // Both devices are read once, so each has values on screen at once when picked again.
    await screen.findByRole("switch", { name: "LED on" });
    await settle();
    fireEvent.click(screen.getByText("B Mouse"));
    await screen.findByRole("heading", { level: 2, name: "B Mouse" });
    await settle();
    fireEvent.click(screen.getByText("A Mouse"));
    await screen.findByRole("heading", { level: 2, name: "A Mouse" });
    await settle();

    // On A the user stages a change: Apply appears.
    fireEvent.click(screen.getByRole("switch", { name: "LED on" }));
    await screen.findByRole("button", { name: "Apply" });

    // B is picked and its read stays out: B's values from the last read are on screen. A's
    // change is not among them, and there is nothing to Apply on B.
    let landB!: (d: PeripheralDetail) => void;
    api.peripheral.mockImplementationOnce(
      () =>
        new Promise<PeripheralDetail>((r) => {
          landB = r;
        }),
    );
    fireEvent.click(screen.getByText("B Mouse"));
    await screen.findByRole("heading", { level: 2, name: "B Mouse" });
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(screen.getByRole("switch", { name: "LED on" }).getAttribute("aria-checked")).toBe("false");
    await act(async () => {
      landB(b);
      await Promise.resolve();
    });
    await settle();

    // An edit on B is B's alone: Apply sends only B's change, to B.
    fireEvent.click(screen.getByRole("switch", { name: "Breathe" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply" }));
    await waitFor(() => expect(api.patchPeripheral).toHaveBeenCalledTimes(1));
    const [sentTo, sent] = api.patchPeripheral.mock.calls[0]!;
    expect(sentTo).toBe("x6:b");
    expect(Object.keys(sent as object)).toEqual(["breathe"]);
  });
});

describe("a read that was out when a write failed", () => {
  it("keeps an edit made after the failure", async () => {
    const device = mouse("x6:late", "Late Mouse");
    api.peripherals.mockResolvedValue({ peripherals: [device.info] });
    api.peripheral.mockResolvedValue(device);
    const first = page();
    await screen.findByRole("switch", { name: "LED on" });
    await settle();
    first.unmount();

    // Second visit: cached values on screen, the fresh read is out.
    let land!: (d: PeripheralDetail) => void;
    api.peripheral.mockImplementationOnce(
      () =>
        new Promise<PeripheralDetail>((r) => {
          land = r;
        }),
    );
    api.patchPeripheral.mockRejectedValueOnce(new Error("the write failed"));
    page();
    fireEvent.click(await screen.findByRole("switch", { name: "LED on" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply" }));
    await screen.findByText("the write failed");

    // After the failure the user makes another edit; then the read that was out lands.
    fireEvent.click(screen.getByRole("switch", { name: "Breathe" }));
    await act(async () => {
      land(device);
      await Promise.resolve();
      await Promise.resolve();
    });

    // The edit made after the failure is still staged.
    expect(screen.queryByRole("button", { name: "Apply" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Breathe" }).getAttribute("aria-checked")).toBe("true");
  });
});


describe("unsent edits", () => {
  it("survive a switch to another device and back", async () => {
    const a = mouse("x6:keep-a", "Keep A");
    const b = mouse("x6:keep-b", "Keep B");
    api.peripherals.mockResolvedValue({ peripherals: [a.info, b.info] });
    api.peripheral.mockImplementation((id: string) => Promise.resolve(id === a.info.id ? a : b));
    page();
    fireEvent.click(await screen.findByRole("switch", { name: "LED on" }));
    await screen.findByRole("button", { name: "Apply" });

    fireEvent.click(screen.getByText("Keep B"));
    await screen.findByRole("heading", { level: 2, name: "Keep B" });
    await settle();
    fireEvent.click(screen.getByText("Keep A"));
    await screen.findByRole("heading", { level: 2, name: "Keep A" });
    await settle();

    // A's read landed again; the edit made before it is still staged.
    expect(screen.queryByRole("button", { name: "Apply" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "LED on" }).getAttribute("aria-checked")).toBe("true");
  });

  it("made while Apply is sending stay staged when it lands", async () => {
    const device = mouse("x6:inflight", "In Flight");
    api.peripherals.mockResolvedValue({ peripherals: [device.info] });
    api.peripheral.mockResolvedValue(device);
    let landPatch!: () => void;
    api.patchPeripheral.mockImplementationOnce(
      () =>
        new Promise<void>((r) => {
          landPatch = () => r();
        }),
    );
    page();
    fireEvent.click(await screen.findByRole("switch", { name: "LED on" }));
    await settle();
    fireEvent.click(await screen.findByRole("button", { name: "Apply" }));
    await waitFor(() => expect(api.patchPeripheral).toHaveBeenCalledTimes(1));

    // While the change is on its way, the user changes another setting.
    fireEvent.click(screen.getByRole("switch", { name: "Breathe" }));
    // The first change lands; the device now holds LED on.
    api.peripheral.mockResolvedValue({
      ...device,
      values: { ...device.values, "led-on": { type: "bool", value: true } },
    });
    await act(async () => {
      landPatch();
      await Promise.resolve();
    });
    await settle();

    // The edit made while it was out is still staged.
    expect(screen.getByRole("switch", { name: "Breathe" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByText(/1 change not sent/)).toBeTruthy();
  });
});

describe("a failed access change", () => {
  it("leaves the unsent edits it did not carry", async () => {
    const device = mouse("x6:access", "Access Mouse");
    api.peripherals.mockResolvedValue({ peripherals: [device.info] });
    api.peripheral.mockResolvedValue(device);
    api.putPeripheralAccess.mockRejectedValue(new Error("the access was not saved"));
    page();
    fireEvent.click(await screen.findByRole("switch", { name: "LED on" }));
    await settle();
    await screen.findByRole("button", { name: "Apply" });

    fireEvent.click(screen.getByRole("switch", { name: "Change settings on the device" }));
    await screen.findByText("the access was not saved");

    // The next read of the device lands: the LED edit is still staged.
    fireEvent.click(screen.getByRole("button", { name: /Read again/ }));
    await settle();
    await settle();
    expect(screen.getByRole("switch", { name: "LED on" }).getAttribute("aria-checked")).toBe("true");
  });
});
