import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { PeripheralsView } from "@/views/PeripheralsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

// Issue #151: a device read that lands after a failed write must not clear the write's
// message. The user would think the change went through, and nothing says it did not.

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
          properties: [{ key: "led-on", label: "LED on", kind: { type: "bool", default: false } }],
        },
      ],
      read_only: [],
      unreported: [],
    },
    values: { "led-on": { type: "bool", value: false } },
    access: { reads: true, writes: true },
    problem: null,
  };
}

/** The page, with the strip it draws its actions into. */
function page() {
  return render(
    <ToastProvider>
      <StripSlotsProvider>
        <header data-testid="strip">
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </header>
        <PeripheralsView />
      </StripSlotsProvider>
    </ToastProvider>,
  );
}

/** A read that is out, and the key that lands it. */
function pendingRead() {
  let resolve!: (d: PeripheralDetail) => void;
  api.peripheral.mockImplementationOnce(
    () =>
      new Promise<PeripheralDetail>((r) => {
        resolve = r;
      }),
  );
  return (device: PeripheralDetail) => resolve(device);
}

/**
 * The page opened twice. The first visit reads the device and leaves its values in the
 * cache (`lastRead`, PeripheralsView.tsx:82); the second visit shows those values at once,
 * with the fresh read of that visit still out. Returns the switch of the setting and the
 * key that lands the read that is out.
 */
async function openWithCachedDetail(device: PeripheralDetail) {
  api.peripherals.mockResolvedValue({ peripherals: [device.info] });
  api.peripheral.mockResolvedValue(device);
  const first = page();
  // The first read has landed: the values are in the cache now.
  await screen.findByRole("switch", { name: "LED on" });
  await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(1));
  await act(async () => {
    await Promise.resolve();
  });
  first.unmount();

  // The second visit: the cached values are on screen and the fresh read is out.
  const land = pendingRead();
  page();
  const led = await screen.findByRole("switch", { name: "LED on" });
  await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(2));
  return { led, land: () => land(device) };
}

/** The page with a Read again that is out: the device is read again on purpose. */
async function openWithReadAgainOut(device: PeripheralDetail) {
  api.peripherals.mockResolvedValue({ peripherals: [device.info] });
  api.peripheral.mockResolvedValue(device);
  page();
  const led = await screen.findByRole("switch", { name: "LED on" });
  await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(1));
  await act(async () => {
    await Promise.resolve();
  });
  // The user asks for the device to be read again: that read is out now.
  const land = pendingRead();
  fireEvent.click(screen.getByRole("button", { name: "Read again" }));
  await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(2));
  return { led, land: () => land(device) };
}

/** The user changes the setting and sends it. The write fails. */
async function changeAndFailToSend(led: HTMLElement, message: string) {
  fireEvent.click(led);
  fireEvent.click(await screen.findByRole("button", { name: "Apply" }));
  await waitFor(() => expect(api.patchPeripheral).toHaveBeenCalledTimes(1));
  await screen.findByText(message);
}

/** The read that was out lands. */
async function landTheRead(land: () => void) {
  await act(async () => {
    land();
    await Promise.resolve();
    await Promise.resolve();
  });
  // The read has landed: the pending change went with it.
  await waitFor(() => expect(screen.queryByRole("button", { name: "Apply" })).toBeNull());
}

describe("a read that lands after a failed write", () => {
  it("keeps the failed write's message on screen", async () => {
    const device = mouse("x6:late", "Late Mouse");
    api.patchPeripheral.mockRejectedValue(new Error("the device did not take the change"));
    const { led, land } = await openWithCachedDetail(device);

    // The user changes a setting while the fresh read is out, and sends it: it fails.
    await changeAndFailToSend(led, "the device did not take the change");

    // The fresh read lands now, after the failure.
    await landTheRead(land);

    // The message of the failed write is still on screen: the change did not happen.
    expect(screen.queryByText("the device did not take the change")).toBeTruthy();
  });

  it("keeps it when the read that lands is a Read again", async () => {
    const device = mouse("x6:again", "Again Mouse");
    api.patchPeripheral.mockRejectedValue(new Error("the device did not take the change"));
    const { led, land } = await openWithReadAgainOut(device);

    await changeAndFailToSend(led, "the device did not take the change");

    // The Read again that was out lands now, after the failure.
    await landTheRead(land);

    // The message of the failed write is still on screen: the change did not happen.
    expect(screen.queryByText("the device did not take the change")).toBeTruthy();
  });
});

describe("what does clear the failed write's message", () => {
  it("a Read again that comes after it", async () => {
    const device = mouse("x6:after-read", "After Read Mouse");
    api.patchPeripheral.mockRejectedValue(new Error("the device did not take the change"));
    const { led, land } = await openWithCachedDetail(device);

    await changeAndFailToSend(led, "the device did not take the change");

    // The read that was out lands first.
    await landTheRead(land);

    // The user reads the device again: that is a new action, and it clears the message.
    fireEvent.click(screen.getByRole("button", { name: "Read again" }));
    await waitFor(() => expect(api.peripheral).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(screen.queryByText("the device did not take the change")).toBeNull());
  });

  it("a write that comes after it and lands", async () => {
    const device = mouse("x6:after-write", "After Write Mouse");
    api.patchPeripheral
      .mockRejectedValueOnce(new Error("the device did not take the change"))
      .mockResolvedValue({ ok: true });
    const { led, land } = await openWithCachedDetail(device);

    await changeAndFailToSend(led, "the device did not take the change");

    // The read that was out lands first.
    await landTheRead(land);

    // The user sends the change again, and this time the device takes it.
    fireEvent.click(led);
    fireEvent.click(await screen.findByRole("button", { name: "Apply" }));
    await waitFor(() => expect(api.patchPeripheral).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText("the device did not take the change")).toBeNull());
  });
});
