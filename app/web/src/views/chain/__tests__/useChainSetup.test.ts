// The chain setup flow's data layer, against its own answers. The routes are the spine's
// (`crates/canvas-proto/src/api.rs`, `routes::CHAIN_*`); the daemon that serves them is written
// against the same spine, so these tests answer `api` themselves and check what the hook sends:
// the session's lifetime, the throttle on the light calls, the heartbeat that keeps the session
// alive, and the two writes that end it (commit and empty).

import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { api } = vi.hoisted(() => ({
  api: {
    chains: {
      list: vi.fn(),
      start: vi.fn(),
      light: vi.fn(),
      cancel: vi.fn(),
      empty: vi.fn(),
      commit: vi.fn(),
    },
    lights: { list: vi.fn() },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { ApiFailure } from "@/lib/errors";
import { patchSnap } from "@/lib/store";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { ChainView } from "@/proto/ChainView";
import type { LightView } from "@/proto/LightView";
import type { SetupLight } from "@/proto/SetupLight";
import { PART_COLOURS } from "@/views/chain/partColours";
import { useChainSetup } from "@/views/chain/useChainSetup";

const DEVICE = "openrgb:A1B2C3D4E5F6";
const SESSION = "session-1";

/** The whole-zone light a header has before anything is set up: a ring, so a fan is its guess. */
function wholeZoneLight(): LightView {
  return {
    light: { id: `${DEVICE}#z1`, device_id: DEVICE, zone_index: 1, arrangement: "ring", reversed: false },
    led_count: 120,
    connected: true,
    label: "MSI MYSTIC LIGHT · JARGB 1",
  };
}

/** A header no one has described yet: first run sized it to 120, the device can drive 300. */
function notSetUp(over: Partial<ChainView> = {}): ChainView {
  return {
    device_id: DEVICE,
    device_name: "MSI MYSTIC LIGHT",
    zone_index: 1,
    zone_name: "JARGB 1",
    state: "not_set_up",
    leds: 120,
    min_leds: 0,
    max_leds: 300,
    parts: [wholeZoneLight()],
    ...over,
  };
}

/** One part of a set-up chain, by position (1-based), at `start` along the zone. */
function part(
  position: number,
  name: string,
  leds: number,
  start: number,
  kind: ChainPartKind,
): LightView {
  return {
    light: {
      id: `${DEVICE}#z1p${position}`,
      device_id: DEVICE,
      zone_index: 1,
      leds: { start, count: leds },
      name,
      arrangement: kind === "fan" ? "ring" : "strip",
      reversed: false,
      kind,
    },
    led_count: leds,
    connected: true,
    label: name,
  };
}

/**
 * A header set up as seven things, uneven on purpose: a re-setup must show these lengths, not the
 * even split of the same total. "Top intake" and "Front strip" are names the owner typed.
 */
function setUp(): ChainView {
  const rows: [string, number, ChainPartKind][] = [
    ["Top intake", 16, "fan"],
    ["Fan 2", 15, "fan"],
    ["Fan 3", 14, "fan"],
    ["Fan 4", 14, "fan"],
    ["Fan 5", 13, "fan"],
    ["Front strip", 12, "strip"],
    ["Fan 7", 12, "fan"],
  ];
  let start = 0;
  const parts = rows.map(([name, leds, kind], i) => {
    const view = part(i + 1, name, leds, start, kind);
    start += leds;
    return view;
  });
  return notSetUp({ state: "set_up", leds: start, parts });
}

/** The chains the daemon lists after a first run: 7 fans and 1 strip, set up on 2 headers. */
function answered(): ChainView[] {
  const fans: [string, number][] = [
    ["Fan 1", 14],
    ["Fan 2", 14],
    ["Fan 3", 14],
    ["Fan 4", 14],
    ["Fan 5", 14],
    ["Fan 6", 13],
    ["Fan 7", 13],
  ];
  let start = 0;
  const fanParts = fans.map(([name, leds], i) => {
    const view = part(i + 1, name, leds, start, "fan");
    start += leds;
    return view;
  });
  const strip = part(1, "Front strip", 12, 0, "strip");
  return [
    notSetUp({ state: "set_up", leds: 96, parts: fanParts }),
    notSetUp({ zone_index: 2, zone_name: "JARGB 2", state: "set_up", leds: 12, parts: [strip] }),
    notSetUp({ zone_index: 3, zone_name: "JARGB 3" }),
  ];
}

/** Mounts the hook and lets the start answer land. Fake timers are on: nothing waits on real time. */
async function mount(chain: ChainView = notSetUp()) {
  const view = renderHook(() => useChainSetup(chain));
  await flush();
  return view;
}

/** Lets pending answers land and moves the clock by `ms`. */
async function flush(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** The runs of the newest light request. */
function lastRuns(): SetupLight["runs"] {
  const call = api.chains.light.mock.calls.at(-1) as [string, number, SetupLight] | undefined;
  return call?.[2].runs ?? [];
}

describe("the chain setup flow", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    for (const fn of Object.values(api.chains)) fn.mockReset();
    api.lights.list.mockReset();
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 120, raised_to: 300 });
    api.chains.light.mockResolvedValue({ ok: true });
    api.chains.cancel.mockResolvedValue({ ok: true });
    api.chains.empty.mockResolvedValue({ ok: true });
    api.chains.commit.mockResolvedValue({ lights: [], final_leds: 120 });
    api.chains.list.mockResolvedValue({ chains: [] });
    api.lights.list.mockResolvedValue({ lights: [] });
    patchSnap({ chains: null });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("starts the session on mount and lights one run in the first colour", async () => {
    const { result } = await mount();

    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
    expect(result.current.step).toBe("length");
    expect(result.current.length).toBe(120);
    expect(lastRuns()).toEqual([{ start: 0, count: 120, color: PART_COLOURS[0] }]);
  });

  it("sends at most one light call per 60 ms and always the last value", async () => {
    const { result } = await mount();
    api.chains.light.mockClear();

    // Three drags inside one throttle window: one call, and it carries the last value.
    act(() => {
      result.current.setLength(40);
      result.current.setLength(41);
      result.current.setLength(42);
    });
    await flush(5);
    expect(api.chains.light).not.toHaveBeenCalled();
    await flush(60);
    expect(api.chains.light).toHaveBeenCalledTimes(1);
    expect(lastRuns()).toEqual([{ start: 0, count: 42, color: PART_COLOURS[0] }]);

    // A drag waits its window out, then goes: one call per window, never two.
    act(() => {
      result.current.setLength(50);
    });
    await flush(30);
    expect(api.chains.light).toHaveBeenCalledTimes(1);
    await flush(30);
    expect(api.chains.light).toHaveBeenCalledTimes(2);
    expect(lastRuns()[0]?.count).toBe(50);
  });

  it("the heartbeat resends inside the daemon's timeout", async () => {
    await mount();
    api.chains.light.mockClear();

    // The daemon ends a session 6 s after the last light call: two seconds at a time keeps it open.
    await flush(1999);
    expect(api.chains.light).not.toHaveBeenCalled();
    await flush(1);
    expect(api.chains.light).toHaveBeenCalledTimes(1);
    expect(lastRuns()).toEqual([{ start: 0, count: 120, color: PART_COLOURS[0] }]);
    await flush(2000);
    expect(api.chains.light).toHaveBeenCalledTimes(2);
    await flush(2000);
    expect(api.chains.light).toHaveBeenCalledTimes(3);
  });

  it("unmount cancels the session", async () => {
    const { unmount } = await mount();

    unmount();

    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
  });

  it("pagehide cancels with keepalive", async () => {
    const { unmount } = await mount();

    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });
    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION }, { keepalive: true });

    // One cancel, not two: the session is already ended when the tree goes.
    unmount();
    expect(api.chains.cancel).toHaveBeenCalledTimes(1);
  });

  it("beforeunload cancels with keepalive", async () => {
    await mount();

    act(() => {
      window.dispatchEvent(new Event("beforeunload"));
    });

    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION }, { keepalive: true });
  });

  it("back keeps what was entered", async () => {
    const { result } = await mount();

    act(() => {
      result.current.setLength(80);
    });
    await flush(60);
    act(() => {
      result.current.next();
    });
    expect(result.current.step).toBe("count");

    act(() => {
      result.current.setCount(3);
    });
    expect(result.current.count).toBe(3);
    expect(result.current.parts).toHaveLength(3);
    act(() => {
      result.current.setPartName(0, "Top intake");
      result.current.next();
    });
    expect(result.current.step).toBe("confirm");

    // Back to the count and to the length: both values are still there, and so is the name.
    act(() => {
      result.current.back();
    });
    expect(result.current.step).toBe("count");
    expect(result.current.count).toBe(3);
    act(() => {
      result.current.back();
    });
    expect(result.current.step).toBe("length");
    expect(result.current.length).toBe(80);
    act(() => {
      result.current.next();
      result.current.next();
    });
    expect(result.current.step).toBe("confirm");
    expect(result.current.parts.map((p) => p.name)).toEqual(["Top intake", "Fan 2", "Fan 3"]);

    // Nothing was cancelled on the way back through the steps.
    expect(api.chains.cancel).not.toHaveBeenCalled();
  });

  it("re-setup with the same count starts from the existing parts", async () => {
    const { result } = await mount(setUp());

    expect(result.current.length).toBe(96);
    expect(result.current.step).toBe("length");
    act(() => {
      result.current.next();
    });
    expect(result.current.count).toBe(7);
    act(() => {
      result.current.next();
    });
    expect(result.current.step).toBe("confirm");

    // The names, the kinds and the uneven lengths the owner last confirmed: not an even split.
    expect(result.current.parts.map((p) => p.name)).toEqual([
      "Top intake",
      "Fan 2",
      "Fan 3",
      "Fan 4",
      "Fan 5",
      "Front strip",
      "Fan 7",
    ]);
    expect(result.current.parts.map((p) => p.leds)).toEqual([16, 15, 14, 14, 13, 12, 12]);
    expect(result.current.parts.map((p) => p.kind)).toEqual([
      "fan",
      "fan",
      "fan",
      "fan",
      "fan",
      "strip",
      "fan",
    ]);
    // "Top intake" and "Front strip" are the owner's; "Fan 2" is the app's own name.
    expect(result.current.parts.map((p) => p.named)).toEqual([
      true,
      false,
      false,
      false,
      false,
      true,
      false,
    ]);
  });

  it("capped is true when the length equals max_leds", async () => {
    const { result } = await mount();

    act(() => {
      result.current.setLength(500);
    });
    // The slider stops at the device's own maximum, and says the chain goes on past it.
    expect(result.current.length).toBe(300);
    expect(result.current.capped).toBe(true);

    act(() => {
      result.current.setLength(299);
    });
    expect(result.current.capped).toBe(false);
  });

  it("a failed commit sets error and keeps the step for retry", async () => {
    api.chains.commit.mockRejectedValueOnce(
      new ApiFailure(500, "internal", "the header could not be resized: no reply from OpenRGB"),
    );
    const { result } = await mount();

    act(() => {
      result.current.next();
      result.current.next();
    });
    expect(result.current.step).toBe("confirm");
    await act(async () => {
      await result.current.confirm();
    });
    expect(result.current.step).toBe("confirm");
    expect(result.current.error).toBe("the header could not be resized: no reply from OpenRGB");
    expect(api.chains.commit).toHaveBeenCalledTimes(1);
    expect(api.chains.commit.mock.calls[0]?.[2]).toEqual({
      session: SESSION,
      parts: [{ name: "Fan 1", kind: "fan", leds: 120 }],
    });

    // Retry sends the same parts once more, and this time the daemon takes them.
    api.chains.commit.mockResolvedValueOnce({ lights: [], final_leds: 120 });
    await act(async () => {
      result.current.retry();
    });
    await flush();
    expect(api.chains.commit).toHaveBeenCalledTimes(2);
    expect(result.current.step).toBe("done");
    expect(result.current.error).toBeNull();
  });

  it("nothingLitUp calls empty and reads the chains again", async () => {
    const { result } = await mount();

    await act(async () => {
      await result.current.nothingLitUp();
    });

    expect(api.chains.empty).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(api.chains.list).toHaveBeenCalled();
    expect(result.current.step).toBe("done");
  });

  it("foundSummary reads 7 fans and 1 strip on 2 chains.", async () => {
    api.chains.list.mockResolvedValue({ chains: answered() });
    const { result } = await mount(setUp());

    act(() => {
      result.current.next();
      result.current.next();
    });
    await act(async () => {
      await result.current.confirm();
    });
    await flush();

    expect(result.current.step).toBe("done");
    expect(result.current.foundSummary).toBe("7 fans and 1 strip on 2 chains.");
  });
});
