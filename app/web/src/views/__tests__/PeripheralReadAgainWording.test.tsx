// Issue #374: the "Read again" tooltip was a lower-case run-on with a semicolon, and it said the
// mouse pauses for a few seconds. What the code does is different: the editor asks the daemon
// for the device with `?fresh=true`, the daemon opens the device and reads its reports, and the
// adapter's read is a series of short report exchanges with a pause between them (the X6: five
// reports, a 250 ms wait for each grant and a 500 ms pause between the reports). The device is
// busy with those exchanges while the read runs and is usable again when it ends; it is never
// held still for the whole read. The wording says that, and the busy state says "reading".

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

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

/** One mouse of the kind the app can configure. `read` says it was read already. */
function mouse(id: string, read: boolean): PeripheralDetail {
  return {
    info: {
      id,
      name: "Attack Shark X6",
      vendor: "Beken",
      kind: "mouse",
      adapter: "attack-shark-x6",
      serial: "",
      location: "hid:1d57:fa60",
      connected: true,
    },
    schema: {
      device: id,
      kind: "mouse",
      sections: [{ id: "light", title: "Lighting", properties: [] }],
      read_only: [],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values: read ? { "dpi-stage-count": { type: "number", value: 6 } } : {},
    access: { reads: true, writes: false },
    problem: read ? null : "the device's settings have not been read yet",
    ...(read ? { read_at_ms: Date.now() as unknown as bigint } : {}),
  };
}

/** One controller of the kind the app can configure, read already. */
function pad(id: string): PeripheralDetail {
  return {
    info: {
      id,
      name: "8BitDo Ultimate 2 Wireless (XInput)",
      vendor: "8BitDo",
      kind: "game_controller",
      adapter: "eightbitdo-ultimate-2",
      serial: "",
      location: "hid:2dc8:3106",
      connected: true,
    },
    schema: {
      device: id,
      kind: "game_controller",
      sections: [{ id: "light", title: "Lighting", properties: [] }],
      read_only: [],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values: {},
    access: { reads: true, writes: false },
    problem: null,
    read_at_ms: Date.now() as unknown as bigint,
  };
}

/** The words of the tooltip on the control with this name. */
function tipOf(control: HTMLElement): string {
  return control.closest(".tip")?.querySelector("[role='tooltip']")?.textContent ?? "";
}

/** Every tooltip on the Peripherals page, in the order they stand. */
function tips(): string[] {
  return [...document.querySelectorAll(".rd-peripherals [role='tooltip']")].map(
    (b) => b.textContent ?? "",
  );
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

describe("what the Peripherals page says about a read (#374)", () => {
  it("the Read again tooltip is two sentences that match what a read does", async () => {
    const id = "x6:wording";
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, true).info] });
    api.peripheral.mockResolvedValue(mouse(id, true));
    render(<PeripheralsView />);
    const key = await screen.findByRole("button", { name: "Read again" });
    const words = tipOf(key);
    expect(words).toBe(
      "Reads the settings from the mouse again, to pick up changes made on the mouse itself. " +
        "The mouse may stutter briefly while it reads.",
    );
    // A sentence, not a run-on: it starts with a capital and it has no semicolon in it.
    expect(words).toMatch(/^[A-Z]/);
    expect(words).not.toContain(";");
  });

  it("the controller's Read again tooltip promises the read and claims no stutter (#387)", async () => {
    const id = "pad:wording";
    api.peripherals.mockResolvedValue({ peripherals: [pad(id).info] });
    api.peripheral.mockResolvedValue(pad(id));
    const view = render(<PeripheralsView />);
    const key = await screen.findByRole("button", { name: "Read again" });
    const words = tipOf(key);
    // The pad's read is 36 chunks of its profile image with no pointer in it, and nothing has
    // measured what its buttons do during one: the tooltip says only what is known.
    expect(words).toBe(
      "Reads the settings from the controller again, to pick up changes made on the controller itself.",
    );
    expect(words).not.toMatch(/stutter/i);
    expect(view.container.ownerDocument.body.textContent ?? "").not.toMatch(/stutter/i);
  });

  it("no wording on the page says the device is paused", async () => {
    const id = "x6:no-pause";
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, true).info] });
    api.peripheral.mockResolvedValue(mouse(id, true));
    render(<PeripheralsView />);
    await screen.findByRole("button", { name: "Read again" });
    const page = document.querySelector(".rd-peripherals");
    expect(page?.textContent ?? "").not.toMatch(/paus/i);
  });

  it("every tooltip on the page is a sentence", async () => {
    const id = "x6:sentences";
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, true).info] });
    api.peripheral.mockResolvedValue(mouse(id, true));
    render(<PeripheralsView />);
    await screen.findByRole("button", { name: "Read again" });
    const all = tips();
    expect(all.length).toBeGreaterThan(0);
    for (const words of all) {
      expect(words).toMatch(/^[A-Z]/);
      expect(words).not.toContain(";");
      expect(words.endsWith(".")).toBe(true);
    }
  });

  it("the frame that stands for a device nothing was read from says what reading does", async () => {
    const id = "x6:not-read";
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, false).info] });
    api.peripheral.mockResolvedValue(mouse(id, false));
    render(<PeripheralsView />);
    const frame = await screen.findByText(
      "Not read yet. Press Read again to read the mouse's settings.",
    );
    expect(frame.textContent).not.toMatch(/paus/i);
  });

  it("the busy state says reading, and it ends when the read ends", async () => {
    const id = "x6:busy";
    let land: (d: PeripheralDetail) => void = () => {};
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, true).info] });
    api.peripheral.mockImplementation((_id: string, opts?: { fresh?: boolean }) => {
      // The Read again is out: the daemon has not answered it yet.
      if (opts?.fresh) return new Promise<PeripheralDetail>((resolve) => (land = resolve));
      return Promise.resolve(mouse(id, true));
    });
    api.peripheralRead.mockResolvedValue({ stage: "reading", done: 2, total: 5 });
    render(<PeripheralsView />);
    const key = await screen.findByRole("button", { name: "Read again" });
    fireEvent.click(key);

    // While the read runs the page says what it is doing, and it says reading.
    const stage = await screen.findByText("Reading settings · 2 of 5");
    expect(stage).toBeTruthy();
    expect(document.querySelector(".rd-peripherals")?.textContent ?? "").not.toMatch(/paus/i);

    // The read ends: the busy state ends with it.
    await act(async () => {
      land(mouse(id, true));
    });
    await waitFor(() => expect(document.querySelector(".load-stage")).toBeNull());
  });
});
