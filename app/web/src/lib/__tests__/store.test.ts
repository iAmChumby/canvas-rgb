import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The store's one way to the daemon. The tests here answer it themselves, so an answer can be
// slow, late or never.

const { api } = vi.hoisted(() => ({
  api: {
    status: vi.fn(),
    fans: vi.fn(),
    devices: vi.fn(),
    effects: vi.fn(),
    layouts: vi.fn(),
    render: vi.fn(),
    settings: vi.fn(),
    displays: vi.fn(),
    scenes: { list: vi.fn() },
    lights: { list: vi.fn() },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import {
  getSnap,
  patchSnap,
  refreshDevices,
  refreshDisplays,
  refreshEffects,
  refreshFans,
  refreshLayouts,
  refreshLights,
  refreshScenes,
  refreshSettings,
  refreshStatus,
  retryEffects,
  retryScenes,
} from "@/lib/store";
import { ApiFailure } from "@/lib/errors";

const STATUS = { version: "0.2.0", devices_connected: 3, devices_total: 4 };

describe("the store's answers", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) {
      if (typeof fn === "function") fn.mockReset();
    }
    api.scenes.list.mockReset();
    api.lights.list.mockReset();
    patchSnap({
      status: null,
      statusError: null,
      fans: null,
      fansError: null,
      devices: null,
      devicesError: null,
      devicesTotal: null,
      effects: null,
      effectsError: null,
      layouts: null,
      layoutsError: null,
      settings: null,
      settingsError: null,
      displays: null,
      displaysError: null,
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

  it("stops asking a daemon without the scene routes until its version changes", async () => {
    // The first status read sets this daemon's version, so the test does not depend on an earlier
    // test having set one.
    api.status.mockResolvedValue(STATUS);
    await refreshStatus();
    api.scenes.list.mockRejectedValue(new ApiFailure(404, "not_found", "no route GET /api/v1/scenes"));
    api.lights.list.mockRejectedValue(new ApiFailure(404, "not_found", "no route GET /api/v1/lights"));

    await refreshScenes();
    await refreshLights();
    await refreshStatus();
    expect(api.scenes.list).toHaveBeenCalledTimes(1);
    expect(api.lights.list).toHaveBeenCalledTimes(1);

    // The poll asks again: a daemon that has no scene routes says the same.
    await refreshScenes();
    await refreshLights();
    expect(api.scenes.list).toHaveBeenCalledTimes(1);
    expect(api.lights.list).toHaveBeenCalledTimes(1);

    // A Try again asks again: the user says so.
    await retryScenes();
    expect(api.scenes.list).toHaveBeenCalledTimes(2);
    expect(api.lights.list).toHaveBeenCalledTimes(2);

    // A daemon that changed may have the routes.
    api.status.mockResolvedValue({ ...STATUS, version: "0.14.0" });
    await refreshStatus();
    await refreshScenes();
    await refreshLights();
    expect(api.scenes.list).toHaveBeenCalledTimes(3);
    expect(api.lights.list).toHaveBeenCalledTimes(3);
  });

  it("asks the scene routes again when the daemon comes back after a failed status read", async () => {
    api.status.mockResolvedValue(STATUS);
    await refreshStatus();
    api.scenes.list.mockRejectedValue(new ApiFailure(404, "not_found", "no route GET /api/v1/scenes"));
    await refreshScenes();
    expect(api.scenes.list).toHaveBeenCalledTimes(1);

    // The daemon is down for one read, then answers again at the same version: a rebuilt dev
    // daemon keeps its version, and it may have the routes now.
    api.status.mockRejectedValueOnce(new Error("connection refused"));
    await refreshStatus();
    await refreshStatus();
    await refreshScenes();
    expect(api.scenes.list).toHaveBeenCalledTimes(2);
  });

  it("asks the scene routes again when the daemon restarted without a failed read", async () => {
    // A daemon restarted inside one poll gap answers every read, so no read fails, and a rebuilt
    // dev daemon keeps its version: only its frame count starts again from zero. The version
    // starts fresh here, so the test does not depend on an earlier one's gate.
    api.status.mockResolvedValue({ ...STATUS, version: "0.14.1", frames_rendered: 1_000_000 });
    await refreshStatus();
    api.scenes.list.mockRejectedValue(new ApiFailure(404, "not_found", "no route GET /api/v1/scenes"));
    await refreshScenes();
    expect(api.scenes.list).toHaveBeenCalledTimes(1);

    // The next read is answered by the daemon that replaced it: same version, frames from zero.
    api.status.mockResolvedValue({ ...STATUS, version: "0.14.1", frames_rendered: 40 });
    await refreshStatus();
    await refreshScenes();
    expect(api.scenes.list).toHaveBeenCalledTimes(2);
  });
});

/**
 * Every area the store reads, with the answer it takes and the field a failure lands in.
 * One table, one test: a failure that arrives after a newer answer has landed must not be
 * shown, in any of the areas.
 */
const AREAS = [
  { name: "status", call: () => refreshStatus(), answer: STATUS, field: "statusError" },
  { name: "fans", call: () => refreshFans(), answer: { headers: [] }, field: "fansError" },
  { name: "devices", call: () => refreshDevices(), answer: { devices: [] }, field: "devicesError" },
  {
    name: "effects",
    call: () => refreshEffects(),
    answer: { effects: [], active: null, failed: [] },
    field: "effectsError",
  },
  {
    name: "layouts",
    call: () => refreshLayouts(),
    answer: { layouts: [], active: "" },
    field: "layoutsError",
  },
  { name: "settings", call: () => refreshSettings(), answer: { fps: 30 }, field: "settingsError" },
  { name: "displays", call: () => refreshDisplays(), answer: [], field: "displaysError" },
] as const;

describe("the store's late failures", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) {
      if (typeof fn === "function") fn.mockReset();
    }
    patchSnap({
      statusError: null,
      fansError: null,
      devicesError: null,
      effectsError: null,
      layoutsError: null,
      settingsError: null,
      displaysError: null,
    });
  });

  it.each(AREAS)("drops a late failure for the $name area", async (area) => {
    let fail!: (e: unknown) => void;
    api[area.name].mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          fail = reject;
        }),
    );
    api[area.name].mockImplementationOnce(() => Promise.resolve(area.answer));
    const late = area.call();
    // The newer answer lands first.
    await area.call();
    expect(getSnap()[area.field]).toBeNull();
    // The older one fails afterwards: it must not land on a page that has moved on.
    fail(new Error("the service is late and says no"));
    await late;
    expect(getSnap()[area.field]).toBeNull();
  });
});
