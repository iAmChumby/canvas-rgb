import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The store's one way to the daemon. The tests here answer it themselves, so an answer can be
// slow, late or never.

const { api } = vi.hoisted(() => ({
  api: {
    status: vi.fn(),
    effects: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { getSnap, patchSnap, refreshEffects, refreshStatus, retryEffects } from "@/lib/store";

const STATUS = { version: "0.2.0", devices_connected: 3, devices_total: 4 };

describe("the store's answers", () => {
  beforeEach(() => {
    api.status.mockReset();
    api.effects.mockReset();
    patchSnap({
      status: null,
      statusError: null,
      effects: null,
      effectsError: null,
      pending: [],
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("keeps an answer that is slower than the poll", async () => {
    vi.useFakeTimers();
    // Every answer takes longer than the poll interval, so a newer request is always on its
    // way when the older one lands. The older one must still reach the page.
    api.status.mockImplementation(
      () =>
        new Promise((resolve) => {
          setTimeout(() => resolve(STATUS), 1500);
        }),
    );
    const poll = setInterval(() => {
      void refreshStatus();
    }, 1000);
    await vi.advanceTimersByTimeAsync(5000);
    clearInterval(poll);
    expect(getSnap().status).not.toBeNull();
  });

  it("does not start a second load while the first one is running", async () => {
    let release!: () => void;
    api.effects.mockImplementation(
      () =>
        new Promise((resolve) => {
          release = () => resolve({ effects: [], active: null, failed: [] });
        }),
    );
    const first = retryEffects();
    const second = retryEffects();
    // The button cannot be pressed twice: one load is in flight, not two.
    expect(api.effects).toHaveBeenCalledTimes(1);
    release();
    await Promise.all([first, second]);
    expect(getSnap().pending).toEqual([]);
  });

  it("drops a failure that arrives after a newer answer has landed", async () => {
    let fail!: (e: unknown) => void;
    api.effects.mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          fail = reject;
        }),
    );
    api.effects.mockImplementationOnce(() =>
      Promise.resolve({ effects: [{ id: "aurora", name: "Aurora" }], active: null, failed: [] }),
    );
    const late = refreshEffects();
    // The newer answer lands first.
    await refreshEffects();
    expect(getSnap().effectsError).toBeNull();
    // The older one fails afterwards: it must not land on a page that has moved on.
    fail(new Error("the service is late and says no"));
    await late;
    expect(getSnap().effectsError).toBeNull();
    expect(getSnap().effects).not.toBeNull();
  });
});
