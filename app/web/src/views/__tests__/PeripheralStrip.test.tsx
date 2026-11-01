import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { PeripheralsView } from "@/views/PeripheralsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

// The peripherals page keeps its pending edits and the way to send them in the status strip
// (R4): the count, Revert and Apply stand there while edits are staged, and are gone when none are.

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

/** A mouse that may be changed, with one switch in a section of its own. */
const mouse: PeripheralDetail = {
  info: {
    id: "x6:one",
    name: "Attack Shark X6",
    vendor: "Attack Shark",
    kind: "mouse",
    adapter: "attack-shark-x6",
    serial: "",
    location: "",
    connected: true,
  },
  schema: {
    device: "x6:one",
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

describe("the pending edits in the strip", () => {
  it("count the edits, Revert drops them, and Apply sends them", async () => {
    api.peripherals.mockResolvedValue({ peripherals: [mouse.info] });
    api.peripheral.mockResolvedValue(mouse);
    api.patchPeripheral.mockResolvedValue({ ok: true });
    page();
    const strip = screen.getByTestId("strip");
    const led = await screen.findByRole("switch", { name: "LED on" });

    // Nothing is staged: the strip has no Revert and no Apply.
    expect(screen.queryByRole("button", { name: "Revert" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();

    // One edit: the strip says so, and holds Revert and Apply.
    fireEvent.click(led);
    expect(await screen.findByText("1 change not sent")).toBeTruthy();
    expect(strip.contains(screen.getByRole("button", { name: "Revert" }))).toBe(true);
    expect(strip.contains(screen.getByRole("button", { name: "Apply" }))).toBe(true);

    // Revert: the switch goes back, and the strip's keys are gone.
    fireEvent.click(screen.getByRole("button", { name: "Revert" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Apply" })).toBeNull());
    expect(screen.getByRole("switch", { name: "LED on" }).getAttribute("aria-checked")).toBe("false");
    expect(api.patchPeripheral).not.toHaveBeenCalled();

    // The edit again, and Apply: the change goes to the device, and the strip's keys are gone.
    fireEvent.click(screen.getByRole("switch", { name: "LED on" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply" }));
    await waitFor(() => expect(api.patchPeripheral).toHaveBeenCalledTimes(1));
    expect(api.patchPeripheral).toHaveBeenCalledWith("x6:one", { "led-on": { type: "bool", value: true } });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Apply" })).toBeNull());
  });
});
