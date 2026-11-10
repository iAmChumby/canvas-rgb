import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView, UNREAD_POLL_MS } from "@/views/PeripheralsView";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

// Issue #127: opening Peripherals reads nothing from the device. Until the daemon has read it,
// the page says so instead of showing settings it made up, and asks again every few seconds.

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

const NOT_READ_TEXT =
  "Not read yet. canvas-rgb reads it while you're not using the mouse, or press Read again (this pauses the mouse for a few seconds).";

/** A mouse with a DPI section; `readAt` is when the daemon read it (Unix ms), or null for never. */
function mouse(id: string, readAt: number | null): PeripheralDetail {
  const read = readAt !== null;
  return {
    info: {
      id,
      name: "Attack Shark X6",
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
          id: "dpi",
          title: "DPI",
          properties: [
            {
              key: "dpi-stage-count",
              label: "Stages",
              description: "",
              kind: { type: "number", min: 1, max: 6, step: 1, default: 3 },
            },
            {
              key: "dpi-stage-1",
              label: "DPI stage 1",
              description: "",
              kind: { type: "number", min: 100, max: 26000, step: 50, default: 800 },
            },
          ],
        },
      ],
      read_only: [],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values: read
      ? {
          "dpi-stage-count": { type: "number", value: 1 },
          "dpi-stage-1": { type: "number", value: 1600 },
        }
      : {},
    access: { reads: true, writes: false },
    problem: read ? null : "the device's settings have not been read yet",
    // The daemon sends a plain number; the generated type says bigint for a u64.
    ...(read ? { read_at_ms: readAt as unknown as bigint } : {}),
  };
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

/** Serves one device: the list and its detail. */
function serve(id: string, detail: () => Promise<PeripheralDetail>) {
  api.peripherals.mockResolvedValue({ peripherals: [mouse(id, null).info] });
  api.peripheral.mockImplementation(detail);
}

/** Lets the page's promises settle and the clock run `ms`. */
async function tick(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

describe("a device nothing was read from", () => {
  it("shows a frame that says so, with no settings made up, and Read again as the primary key", async () => {
    serve("x6:unread-frame", async () => mouse("x6:unread-frame", null));
    render(<PeripheralsView />);
    const note = await screen.findByText(NOT_READ_TEXT);
    expect(note).toBeTruthy();
    // No DPI chips, no controls: the frame stands where they will be.
    expect(document.querySelector(".dpi-chip")).toBeNull();
    expect(document.querySelector(".rd-peri-sections")).toBeNull();
    // The daemon's own "not read yet" problem line is the same words: it is not shown twice.
    expect(document.querySelector('[data-role="problem"]')).toBeNull();
    const key = screen.getByRole("button", { name: "Read again" });
    expect(key.className).toContain("btn-primary");
    // The read-time slot holds the state in the head.
    expect(document.querySelector('[data-role="read-at"]')?.textContent).toContain("Not read yet");
  });

  it("is not remembered as a read: picking the device again waits for the daemon", async () => {
    serve("x6:unread-cache", async () => mouse("x6:unread-cache", null));
    render(<PeripheralsView />);
    await screen.findByText(NOT_READ_TEXT);
    cleanup();

    // The daemon is slow this time. A remembered "unread" detail would show the frame at once,
    // marked as a read; the page has to show its placeholder instead.
    api.peripheral.mockImplementation(() => new Promise<PeripheralDetail>(() => {}));
    render(<PeripheralsView />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText(NOT_READ_TEXT)).toBeNull();
    expect(document.querySelector('[data-role="not-read"]')).toBeNull();
  });

  it("asks again every five seconds with a plain GET, and stops when the values arrive", async () => {
    vi.useFakeTimers();
    const id = "x6:unread-poll";
    let answer: PeripheralDetail = mouse(id, null);
    serve(id, async () => answer);
    render(<PeripheralsView />);
    await tick(50);
    expect(screen.queryByText(NOT_READ_TEXT)).not.toBeNull();
    const asked = () => api.peripheral.mock.calls.length;
    const first = asked();

    await tick(UNREAD_POLL_MS);
    expect(asked()).toBe(first + 1);
    await tick(UNREAD_POLL_MS);
    expect(asked()).toBe(first + 2);
    // Every ask is the plain one: it never reads the device.
    for (const call of api.peripheral.mock.calls) {
      expect(call).toEqual([id]);
    }

    // The daemon read the device while the person was away.
    answer = mouse(id, Date.now() - 30_000);
    await tick(UNREAD_POLL_MS);
    expect(screen.queryByText(NOT_READ_TEXT)).toBeNull();
    expect(document.querySelector(".dpi-chip")).not.toBeNull();
    const done = asked();

    // Nothing asks again once there is something to show.
    await tick(UNREAD_POLL_MS * 4);
    expect(asked()).toBe(done);
  });

  it("stops asking when the page goes away", async () => {
    vi.useFakeTimers();
    const id = "x6:unread-unmount";
    serve(id, async () => mouse(id, null));
    const { unmount } = render(<PeripheralsView />);
    await tick(50);
    await tick(UNREAD_POLL_MS);
    const before = api.peripheral.mock.calls.length;
    unmount();
    await tick(UNREAD_POLL_MS * 4);
    expect(api.peripheral.mock.calls.length).toBe(before);
  });
});

describe("a device that was read", () => {
  it("says when, near Read again: 'Read 3 min ago'", async () => {
    const id = "x6:read-3min";
    serve(id, async () => mouse(id, Date.now() - 3 * 60_000 - 5_000));
    render(<PeripheralsView />);
    await screen.findByRole("button", { name: "Read again" });
    const slot = document.querySelector('[data-role="read-at"]');
    expect(slot?.textContent).toContain("Read 3 min ago");
    expect(slot?.getAttribute("data-stale")).toBe("false");
    expect(slot?.getAttribute("data-empty")).toBe("false");
    // Read again is the ordinary key now, and the settings are on the page.
    expect(screen.getByRole("button", { name: "Read again" }).className).not.toContain("btn-primary");
    expect(screen.queryByText(NOT_READ_TEXT)).toBeNull();
    expect(document.querySelector(".dpi-chip")).not.toBeNull();
  });

  it("is marked stale after thirty minutes", async () => {
    const id = "x6:read-45min";
    serve(id, async () => mouse(id, Date.now() - 45 * 60_000));
    render(<PeripheralsView />);
    await screen.findByRole("button", { name: "Read again" });
    const slot = document.querySelector('[data-role="read-at"]');
    expect(slot?.textContent).toContain("Read 45 min ago");
    expect(slot?.getAttribute("data-stale")).toBe("true");
  });

  it("holds the same wordings in the head as the unread state, so nothing moves", async () => {
    const wordings = (slot: Element | null) =>
      Array.from(slot?.querySelectorAll(".swap > span") ?? []).map((s) => s.textContent);
    const a = "x6:shift-unread";
    serve(a, async () => mouse(a, null));
    const first = render(<PeripheralsView />);
    await screen.findByText(NOT_READ_TEXT);
    const unreadCells = wordings(document.querySelector('[data-role="read-at"]'));
    first.unmount();

    const b = "x6:shift-read";
    serve(b, async () => mouse(b, Date.now() - 60_000));
    render(<PeripheralsView />);
    await screen.findByRole("button", { name: "Read again" });
    await act(async () => {
      await Promise.resolve();
    });
    const readCells = wordings(document.querySelector('[data-role="read-at"]'));
    expect(unreadCells).toHaveLength(3);
    expect(readCells).toHaveLength(3);
    // One cell reserves the width of the longest wording in both states.
    expect(readCells[0]).toBe(unreadCells[0]);
    expect(readCells[2]).toBe(unreadCells[2]);
  });
});
