import { act, cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

// Issue #264: with reads off the daemon holds no values, but the page drew the schema's defaults
// (DPI stages, colours, profile) as if they had been read. The sections stay off the page and a
// frame that says what reading does stands in their place, with the Read switch as its action.

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

const MOUSE_OFF_TEXT =
  "canvas-rgb reads this mouse's settings only when you turn Read on; reading pauses the mouse for a few seconds.";
const PAD_OFF_TEXT =
  "canvas-rgb reads this controller's settings only when you turn Read on; reading pauses the controller for a few seconds.";

/**
 * A mouse with a DPI section, a lighting colour and a read-only profile. `reads` is its access;
 * `readAt` is when the daemon read it (Unix ms), or null for never.
 */
function mouse(id: string, reads: boolean, readAt: number | null): PeripheralDetail {
  return {
    info: {
      id,
      name: "Attack Shark X6",
      vendor: "Beken",
      kind: "mouse",
      adapter: "attack-shark-x6",
      serial: "",
      location: "hid:1d57:fa60:usage000b",
      connected: true,
    },
    schema: {
      device: id,
      kind: "mouse",
      sections: [
        {
          id: "dpi",
          title: "DPI",
          properties: [
            {
              key: "dpi-stage-count",
              label: "DPI stages in use",
              kind: { type: "number", min: 1, max: 8, step: 1, default: 6 },
            },
            {
              key: "dpi-stage-1",
              label: "DPI stage 1",
              kind: { type: "number", min: 50, max: 26000, step: 50, default: 800 },
            },
            {
              key: "dpi-stage-1-colour",
              label: "DPI stage 1 colour",
              kind: { type: "color", default: "#ff0000" },
            },
          ],
        },
        {
          id: "lighting",
          title: "Lighting",
          properties: [
            { key: "led-colour", label: "LED colour", kind: { type: "color", default: "#ef3fff" } },
          ],
        },
        {
          id: "profile",
          title: "Profile",
          properties: [{ key: "profile", label: "Profile", kind: { type: "choice", options: ["1"], default: "1" } }],
        },
      ],
      read_only: ["profile"],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values:
      readAt === null
        ? {}
        : {
            "dpi-stage-count": { type: "number", value: 2 },
            "dpi-stage-1": { type: "number", value: 1600 },
            "dpi-stage-1-colour": { type: "color", value: "#ff0000" },
            "led-colour": { type: "color", value: "#ef3fff" },
            profile: { type: "choice", value: "1" },
          },
    access: { reads, writes: false },
    problem: readAt === null ? (reads ? "the device's settings have not been read yet" : "reads are off for this device") : null,
    // The daemon sends a number; the generated type says bigint for a u64.
    ...(readAt === null ? {} : { read_at_ms: readAt as unknown as bigint }),
  };
}

/** The 8BitDo pad: a profile panel and the button map. */
function pad(id: string, reads: boolean, readAt: number | null): PeripheralDetail {
  return {
    info: {
      id,
      name: "8BitDo Ultimate 2 Wireless (XInput)",
      vendor: "8BitDo",
      kind: "game_controller",
      adapter: "eightbitdo-ultimate-2",
      serial: "",
      location: "hid:2dc8:310b",
      connected: true,
    },
    schema: {
      device: id,
      kind: "game_controller",
      sections: [
        {
          id: "profiles",
          title: "Profiles",
          properties: [
            {
              key: "active-profile",
              label: "Active profile",
              kind: { type: "choice", options: ["1", "2"], default: "1" },
            },
          ],
        },
        {
          id: "buttons",
          title: "Button remap (active profile)",
          properties: [
            { key: "button-a", label: "A", kind: { type: "choice", options: ["none", "a"], default: "none" } },
          ],
        },
        {
          id: "profile-1",
          title: "Profile 1",
          properties: [
            {
              key: "profile-1-stick-left-start",
              label: "Left stick range start (%)",
              kind: { type: "number", min: 0, max: 100, step: 1, default: 0 },
            },
          ],
        },
      ],
      read_only: ["active-profile", "button-a"],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values:
      readAt === null
        ? {}
        : {
            "active-profile": { type: "choice", value: "1" },
            "button-a": { type: "choice", value: "a" },
            "profile-1-stick-left-start": { type: "number", value: 7 },
          },
    access: { reads, writes: false },
    problem: readAt === null ? (reads ? "the device's settings have not been read yet" : "reads are off for this device") : null,
    ...(readAt === null ? {} : { read_at_ms: readAt as unknown as bigint }),
  };
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

/** Serves one device: the list and its detail. */
function serve(detail: PeripheralDetail) {
  api.peripherals.mockResolvedValue({ peripherals: [detail.info] });
  api.peripheral.mockResolvedValue(detail);
}

describe("a device with reads off", () => {
  it("says what reading does, and draws no DPI chips, colour or profile", async () => {
    serve(mouse("x6:reads-off", false, null));
    render(<PeripheralsView />);
    // The head is always there; the settings stand behind the frame or not at all.
    await screen.findByRole("switch", { name: "Read settings from this device" });
    // Nothing read, nothing drawn: no chips, no sections, no colour field, no profile row.
    expect(document.querySelector(".dpi-chip")).toBeNull();
    expect(document.querySelector(".rd-peri-sections")).toBeNull();
    expect(screen.queryByText("LED colour")).toBeNull();
    expect(screen.queryByText("Profile")).toBeNull();
    expect(document.querySelector('[data-role="not-read"]')).toBeNull();
    // The frame says what turning Read on does, in the same words the issue asks for.
    expect(await screen.findByText(MOUSE_OFF_TEXT)).toBeTruthy();
    expect(document.querySelector('[data-role="reads-off"]')).not.toBeNull();
    // The daemon's own "reads are off" line is the same news: it is not shown twice.
    expect(document.querySelector('[data-role="problem"]')).toBeNull();
  });

  it("puts the Read switch in the frame, off, and it starts reading", async () => {
    const id = "x6:reads-off-switch";
    let reads = false;
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, false, null).info] });
    api.peripheral.mockImplementation(async () => mouse(id, reads, reads ? Date.now() - 1_000 : null));
    api.putPeripheralAccess.mockImplementation(async () => {
      reads = true;
      return { reads: true, writes: false };
    });
    render(<PeripheralsView />);
    await screen.findByText(MOUSE_OFF_TEXT);
    const frame = document.querySelector('[data-role="reads-off"]') as HTMLElement;
    const key = within(frame).getByRole("switch", { name: "Read this device's settings" });
    expect(key.getAttribute("aria-checked")).toBe("false");
    await act(async () => {
      key.click();
    });
    expect(api.putPeripheralAccess).toHaveBeenCalledWith(id, { reads: true, writes: false });
    // The read that follows puts the values on the page, so the chips are back.
    await screen.findByText("Lighting");
    expect(document.querySelector(".dpi-chip")).not.toBeNull();
    expect(document.querySelector('[data-role="reads-off"]')).toBeNull();
  });

  it("never reads the device on its own: the frame waits for the switch", async () => {
    const id = "x6:reads-off-quiet";
    serve(mouse(id, false, null));
    render(<PeripheralsView />);
    await screen.findByText(MOUSE_OFF_TEXT);
    expect(api.putPeripheralAccess).not.toHaveBeenCalled();
    expect(api.peripheral).not.toHaveBeenCalledWith(id, { fresh: true });
  });

  it("draws no profile panel or button map for a controller", async () => {
    serve(pad("u2:reads-off", false, null));
    render(<PeripheralsView />);
    await screen.findByText(PAD_OFF_TEXT);
    expect(document.querySelector(".rd-peri-sections")).toBeNull();
    expect(document.querySelector(".pad-map")).toBeNull();
    expect(screen.queryByText("Button remap (active profile)")).toBeNull();
    expect(screen.queryByText("Profile 1")).toBeNull();
    // The head still names the device and offers to read it.
    expect(screen.getByRole("switch", { name: "Read settings from this device" })).toBeTruthy();
  });
});

describe("the head line between reads off, not read yet and read", () => {
  const wordings = (slot: Element | null) =>
    Array.from(slot?.querySelectorAll(".swap > span") ?? []).map((s) => s.textContent);
  const empty = () => document.querySelector('[data-role="read-at"]')?.getAttribute("data-empty");

  it("is the same length and holds the same wordings in all three states", async () => {
    serve(mouse("x6:head-off", false, null));
    const off = render(<PeripheralsView />);
    await screen.findByText(MOUSE_OFF_TEXT);
    const offCells = wordings(document.querySelector('[data-role="read-at"]'));
    const offEmpty = empty();
    off.unmount();

    serve(mouse("x6:head-unread", true, null));
    const unread = render(<PeripheralsView />);
    await screen.findByText(/Not read yet\./);
    const unreadCells = wordings(document.querySelector('[data-role="read-at"]'));
    unread.unmount();

    serve(mouse("x6:head-read", true, Date.now() - 60_000));
    render(<PeripheralsView />);
    await screen.findByRole("button", { name: "Read again" });
    const readCells = wordings(document.querySelector('[data-role="read-at"]'));

    expect(offCells).toHaveLength(3);
    expect(unreadCells).toHaveLength(3);
    expect(readCells).toHaveLength(3);
    // One cell reserves the width of the longest wording in every state.
    expect(unreadCells[0]).toBe(offCells[0]);
    expect(readCells[0]).toBe(offCells[0]);
    expect(readCells[2]).toBe(offCells[2]);
    // Reads off keeps the cell, empty, as it does for a daemon that sends no read time.
    expect(offEmpty).toBe("true");
    expect(empty()).toBe("false");
  });
});
