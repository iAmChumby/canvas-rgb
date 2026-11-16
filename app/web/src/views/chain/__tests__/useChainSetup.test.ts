// The header setup flow's data layer, against its own answers. The routes are the spine's
// (`crates/canvas-proto/src/api.rs`, `routes::CHAIN_*`); the daemon that serves them is written
// against the same spine, so these tests answer `api` themselves and check what the hook sends:
// the session's lifetime (it opens on Start, not on mount), the runs each screen lights, the
// throttle on the light calls, the heartbeat that keeps the session alive, the mirror path
// (`copies`), and the two writes that end it (commit and empty).

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
import { TRAIL_COLOUR, useChainSetup, type IntroKind } from "@/views/chain/useChainSetup";

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

/** Mounts the hook. With no intro the session opens at once, as it does from a device's own row. */
async function mount(chain: ChainView = notSetUp(), intro: IntroKind = "none") {
  const view = renderHook(() => useChainSetup(chain, intro));
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

const ORANGE = PART_COLOURS[0];

/** The hook's result, and the step walk the tests use. */
type Hook = Awaited<ReturnType<typeof mount>>;
async function go(view: Hook, ...keys: ("next" | "back")[]): Promise<void> {
  for (const key of keys) {
    act(() => {
      view.result.current[key]();
    });
  }
  await flush(60);
}

describe("the header setup flow", () => {
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

  it("opens the session at once with no intro and lights ONE orange LED", async () => {
    const { result } = await mount();

    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
    expect(result.current.step).toBe("lit");
    expect(result.current.copies).toBe(1);
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);
  });

  it("waits on the intro: nothing is started until start()", async () => {
    const { result } = await mount(notSetUp(), "full");

    expect(result.current.step).toBe("intro");
    await flush(5000);
    expect(api.chains.start).not.toHaveBeenCalled();
    expect(api.chains.light).not.toHaveBeenCalled();
  });

  it("shows Starting until the daemon has answered, and only then the first question", async () => {
    let answer: (v: { session: string; restore_to: number; raised_to: number }) => void = () => {};
    api.chains.start.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const { result } = await mount(notSetUp(), "full");

    act(() => {
      result.current.start();
    });
    await flush();
    expect(result.current.step).toBe("starting");
    expect(api.chains.start).toHaveBeenCalledTimes(1);
    expect(api.chains.light).not.toHaveBeenCalled();

    await act(async () => {
      answer({ session: SESSION, restore_to: 120, raised_to: 300 });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.step).toBe("lit");
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);
  });

  it("Later while the header is still being raised puts it back as soon as the answer lands", async () => {
    let answer: (v: { session: string; restore_to: number; raised_to: number }) => void = () => {};
    api.chains.start.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const { result } = await mount(notSetUp(), "full");
    act(() => {
      result.current.start();
    });
    await flush();

    act(() => {
      result.current.later();
    });
    expect(api.chains.cancel).not.toHaveBeenCalled();
    await act(async () => {
      answer({ session: "late-session", restore_to: 120, raised_to: 300 });
      await vi.advanceTimersByTimeAsync(0);
    });

    // The session that opened after Later is ended at once, and nothing is lit for it.
    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: "late-session" });
    expect(api.chains.light).not.toHaveBeenCalled();
  });

  it("Nothing is plugged in here: starts, then marks the header empty, and shows no question", async () => {
    const { result } = await mount(notSetUp(), "full");

    await act(async () => {
      result.current.startEmpty();
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 1);
    expect(api.chains.empty).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(api.chains.light).not.toHaveBeenCalled();
    expect(result.current.step).toBe("done");
    expect(result.current.doneKind).toBe("empty");
    expect(result.current.doneText).toBe(
      "Marked empty. Set it up again from Devices, then add its lights to your scenes.",
    );
  });

  it("the slide lights a white trail and an orange end, and starts at 1", async () => {
    const { result } = await mount();
    expect(result.current.length).toBe(1);

    await go({ result } as Hook, "next");
    expect(result.current.step).toBe("length");
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);

    act(() => {
      result.current.setLength(8);
    });
    await flush(60);
    expect(lastRuns()).toEqual([
      { start: 0, count: 7, color: TRAIL_COLOUR },
      { start: 7, count: 1, color: ORANGE },
    ]);
  });

  it("sends at most one light call per 60 ms and always the last value", async () => {
    const { result } = await mount();
    await go({ result } as Hook, "next");
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
    expect(lastRuns()).toEqual([
      { start: 0, count: 41, color: TRAIL_COLOUR },
      { start: 41, count: 1, color: ORANGE },
    ]);

    // A drag waits its window out, then goes: one call per window, never two.
    act(() => {
      result.current.setLength(50);
    });
    await flush(30);
    expect(api.chains.light).toHaveBeenCalledTimes(1);
    await flush(30);
    expect(api.chains.light).toHaveBeenCalledTimes(2);
    expect(lastRuns()[1]?.start).toBe(49);
  });

  it("the heartbeat resends inside the daemon's timeout", async () => {
    await mount();
    api.chains.light.mockClear();

    // The daemon ends a session 6 s after the last light call: two seconds at a time keeps it open.
    await flush(1999);
    expect(api.chains.light).not.toHaveBeenCalled();
    await flush(1);
    expect(api.chains.light).toHaveBeenCalledTimes(1);
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);
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
    const view = await mount();
    const { result } = view;

    await go(view, "next");
    act(() => {
      result.current.setLength(80);
    });
    await flush(60);
    await go(view, "next");
    expect(result.current.step).toBe("count");
    // The devices follow the length the owner slid to, even though no count was typed.
    expect(result.current.parts.map((p) => p.leds)).toEqual([80]);

    act(() => {
      result.current.setCount(3);
    });
    expect(result.current.count).toBe(3);
    expect(result.current.parts).toHaveLength(3);
    act(() => {
      result.current.setPartName(0, "Top intake");
    });
    await go(view, "next");
    expect(result.current.step).toBe("check");

    // Back to the count and to the length: both values are still there, and so is the name.
    await go(view, "back");
    expect(result.current.step).toBe("count");
    expect(result.current.count).toBe(3);
    await go(view, "back");
    expect(result.current.step).toBe("length");
    expect(result.current.length).toBe(80);
    await go(view, "next", "next");
    expect(result.current.step).toBe("check");
    expect(result.current.parts.map((p) => p.name)).toEqual(["Top intake", "Fan 2", "Fan 3"]);

    // Nothing was cancelled on the way back through the steps.
    expect(api.chains.cancel).not.toHaveBeenCalled();
  });

  it("re-setup with the same count starts from the existing parts", async () => {
    const view = await mount(setUp());
    const { result } = view;

    expect(result.current.length).toBe(96);
    expect(result.current.step).toBe("lit");
    await go(view, "next");
    expect(result.current.step).toBe("length");
    expect(result.current.length).toBe(96);
    await go(view, "next");
    expect(result.current.count).toBe(7);
    await go(view, "next");
    expect(result.current.step).toBe("check");

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
    // The slider stops at the device's own maximum, and says the header goes on past it.
    expect(result.current.length).toBe(300);
    expect(result.current.capped).toBe(true);

    act(() => {
      result.current.setLength(299);
    });
    expect(result.current.capped).toBe(false);
  });

  it("copies is clamped to 1..32", async () => {
    const { result } = await mount();

    act(() => {
      result.current.setCopies(40);
    });
    expect(result.current.copies).toBe(32);
    act(() => {
      result.current.setCopies(0);
    });
    expect(result.current.copies).toBe(1);
    expect(result.current.mirror).toBe(false);
    act(() => {
      result.current.setCopies(8);
    });
    expect(result.current.mirror).toBe(true);
    // Counting writes nothing to the hardware: the orange LED is the same one.
    expect(api.chains.commit).not.toHaveBeenCalled();
  });

  it("a series commits its devices with no copies and says what it saved", async () => {
    const view = await mount();
    const { result } = view;
    await go(view, "next");
    act(() => {
      result.current.setLength(120);
    });
    await go(view, "next", "next");
    expect(result.current.step).toBe("check");
    await act(async () => {
      await result.current.confirm();
    });

    expect(api.chains.commit.mock.calls[0]?.[2]).toEqual({
      session: SESSION,
      parts: [{ name: "Fan 1", kind: "fan", leds: 120 }],
    });
    expect(result.current.step).toBe("done");
    expect(result.current.doneText).toBe("Saved. 1 device, 120 LEDs on JARGB 1.");
    expect(result.current.doneNote).toMatch(/radiator/);
  });

  it("three 12-LED fans in series commit as three devices", async () => {
    const view = await mount();
    const { result } = view;
    await go(view, "next");
    act(() => {
      result.current.setLength(36);
    });
    await go(view, "next");
    act(() => {
      result.current.setCount(3);
    });
    await go(view, "next");
    // The check lights each device in its own colour.
    expect(lastRuns().map((r) => [r.start, r.count])).toEqual([
      [0, 12],
      [12, 12],
      [24, 12],
    ]);
    await act(async () => {
      await result.current.confirm();
    });

    expect(api.chains.commit.mock.calls[0]?.[2]).toEqual({
      session: SESSION,
      parts: [
        { name: "Fan 1", kind: "fan", leds: 12 },
        { name: "Fan 2", kind: "fan", leds: 12 },
        { name: "Fan 3", kind: "fan", leds: 12 },
      ],
    });
    expect(result.current.doneText).toBe("Saved. 3 devices, 36 LEDs on JARGB 1.");
  });

  it("a mirrored header commits ONE row of 8 LEDs with copies 8, other, and says devices not fans", async () => {
    const view = await mount();
    const { result } = view;
    act(() => {
      result.current.setCopies(8);
    });
    await go(view, "next");
    act(() => {
      result.current.setLength(8);
    });
    await go(view, "next");
    // Straight to the check: a mirror has no devices step.
    expect(result.current.step).toBe("check");
    expect(result.current.mirror).toBe(true);
    // The mirror check lights every LED white, one run.
    expect(lastRuns()).toEqual([{ start: 0, count: 8, color: TRAIL_COLOUR }]);
    expect(result.current.mirrorName).toBe("Same lights, shown 8 times");

    await act(async () => {
      await result.current.confirm();
    });

    expect(api.chains.commit.mock.calls[0]?.[2]).toEqual({
      session: SESSION,
      parts: [{ name: "Same lights, shown 8 times", kind: "other", leds: 8, copies: 8 }],
    });
    expect(result.current.doneText).toBe("Saved. The same 8 LEDs show 8 times on JARGB 1.");
    expect(result.current.doneText).not.toMatch(/fan/i);
    expect(result.current.doneNote).toBe(
      "Devices of different lengths on this header are stored at the length you chose.",
    );
  });

  it("a typed mirror name is the one saved", async () => {
    const view = await mount();
    const { result } = view;
    act(() => {
      result.current.setCopies(8);
    });
    await go(view, "next");
    act(() => {
      result.current.setLength(8);
    });
    await go(view, "next");
    act(() => {
      result.current.setMirrorName("Case lights");
    });
    await act(async () => {
      await result.current.confirm();
    });

    expect(api.chains.commit.mock.calls[0]?.[2]).toEqual({
      session: SESSION,
      parts: [{ name: "Case lights", kind: "other", leds: 8, copies: 8 }],
    });
  });

  it("a re-setup of a mirrored header starts at the stored copies and LEDs, and never keeps the mirror as a row", async () => {
    const mirror = part(1, "Same lights, shown 8 times", 8, 0, "other");
    mirror.light.copies = 8;
    const view = await mount(notSetUp({ state: "set_up", leds: 8, parts: [mirror] }));
    const { result } = view;

    expect(result.current.copies).toBe(8);
    expect(result.current.length).toBe(8);

    // The owner now counts one orange light: the mirror's own row is not carried into a series.
    act(() => {
      result.current.setCopies(1);
    });
    await go(view, "next", "next");
    expect(result.current.parts.map((p) => p.name)).toEqual(["Fan 1"]);
  });

  it("a failed commit sets error and keeps the step for retry", async () => {
    api.chains.commit.mockRejectedValueOnce(
      new ApiFailure(500, "internal", "the header could not be resized: no reply from OpenRGB"),
    );
    const view = await mount();
    const { result } = view;
    await go(view, "next");
    act(() => {
      result.current.setLength(120);
    });
    await go(view, "next", "next");
    expect(result.current.step).toBe("check");
    await act(async () => {
      await result.current.confirm();
    });
    expect(result.current.step).toBe("check");
    expect(result.current.error).toBe("the header could not be resized: no reply from OpenRGB");
    expect(api.chains.commit).toHaveBeenCalledTimes(1);

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

  it("Nothing lit up writes nothing, and Try again lights LED 1 again", async () => {
    const view = await mount();
    const { result } = view;
    api.chains.light.mockClear();

    act(() => {
      result.current.noLight();
    });
    await flush(60);
    expect(result.current.step).toBe("nothing");
    act(() => {
      result.current.cantSee();
    });
    expect(result.current.step).toBe("blind");
    act(() => {
      result.current.tryAgain();
    });
    await flush(60);

    expect(result.current.step).toBe("lit");
    expect(lastRuns()).toEqual([{ start: 0, count: 1, color: ORANGE }]);
    expect(api.chains.empty).not.toHaveBeenCalled();
    expect(api.chains.commit).not.toHaveBeenCalled();
    expect(api.chains.cancel).not.toHaveBeenCalled();
  });

  it("markEmpty calls empty, reads the chains again and says it is marked empty", async () => {
    const { result } = await mount();

    act(() => {
      result.current.noLight();
    });
    let ok = false;
    await act(async () => {
      ok = await result.current.markEmpty();
    });

    expect(ok).toBe(true);
    expect(api.chains.empty).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(api.chains.list).toHaveBeenCalled();
    expect(result.current.step).toBe("done");
    expect(result.current.doneKind).toBe("empty");
  });

  it("a failed mark-empty stays on the checklist with the error, and Retry sends it again", async () => {
    api.chains.empty.mockRejectedValueOnce(new ApiFailure(500, "internal", "the daemon could not write"));
    const { result } = await mount();
    act(() => {
      result.current.noLight();
    });

    let ok = true;
    await act(async () => {
      ok = await result.current.markEmpty();
    });
    expect(ok).toBe(false);
    expect(result.current.step).toBe("nothing");
    expect(result.current.error).toBe("the daemon could not write");
    expect(api.chains.cancel).not.toHaveBeenCalled();

    await act(async () => {
      result.current.retry();
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.chains.empty).toHaveBeenCalledTimes(2);
    expect(result.current.step).toBe("done");
  });

  it("a failed start shows the error, and Retry opens a new session", async () => {
    api.chains.start.mockRejectedValueOnce(new ApiFailure(503, "unavailable", "busy"));
    const { result } = await mount();
    expect(result.current.step).toBe("error");
    expect(result.current.error).toBe("busy");

    await act(async () => {
      result.current.retry();
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.chains.start).toHaveBeenCalledTimes(2);
    expect(result.current.step).toBe("lit");
    expect(result.current.error).toBeNull();
  });

  it("a stale session (409) on a light call ends the flow in the error screen, and Retry starts afresh", async () => {
    const { result } = await mount();
    api.chains.light.mockRejectedValueOnce(new ApiFailure(409, "conflict", "this setup is not open any more"));

    await flush(2000);
    expect(result.current.step).toBe("error");

    await act(async () => {
      result.current.retry();
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.chains.start).toHaveBeenCalledTimes(2);
    expect(result.current.step).toBe("lit");
  });
});
