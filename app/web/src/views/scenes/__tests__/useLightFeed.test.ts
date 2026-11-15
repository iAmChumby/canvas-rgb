import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TauriGlobal } from "@/lib/host";
import { patchSnap } from "@/lib/store";
import { DEMO_LIGHTS, DEMO_SCENES } from "@/lib/fixtures";
import { demoLightColours, useLightFeed } from "@/views/scenes/useLightFeed";
import type { Scene } from "@/proto/Scene";

// The per-light feed: `GET /api/v1/preview/lights` (a WebSocket whose first message maps a u16
// index to a light id, then binary frames: per light a u16 LE LED index, a u16 LE LED count,
// then that many RGB triples).

class FakeSocket {
  static opened: FakeSocket[] = [];
  binaryType = "";
  onopen: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor(readonly url: string) {
    FakeSocket.opened.push(this);
  }
  close(): void {
    this.closed = true;
  }
}

/** One binary frame: each light's index, its LED count, then its RGB triples. */
function frame(parts: { index: number; rgb: number[] }[]): ArrayBuffer {
  const total = parts.reduce((n, p) => n + 4 + p.rgb.length, 0);
  const buf = new ArrayBuffer(total);
  const view = new DataView(buf);
  const bytes = new Uint8Array(buf);
  let at = 0;
  for (const part of parts) {
    view.setUint16(at, part.index, true);
    view.setUint16(at + 2, part.rgb.length / 3, true);
    bytes.set(part.rgb, at + 4);
    at += 4 + part.rgb.length;
  }
  return buf;
}

beforeEach(() => {
  FakeSocket.opened = [];
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeSocket as unknown as typeof WebSocket;
  window.__TAURI__ = {
    core: { invoke: async () => ({ base_url: "http://127.0.0.1:9430", token: "tok" }) },
    event: { listen: async () => () => undefined },
  } as unknown as TauriGlobal;
});

afterEach(() => {
  vi.useRealTimers();
  delete window.__TAURI__;
});

describe("useLightFeed", () => {
  it("decodes the index message then frames", async () => {
    const { result, unmount } = renderHook(() => useLightFeed());
    await waitFor(() => expect(FakeSocket.opened).toHaveLength(1));
    const socket = FakeSocket.opened[0]!;
    expect(socket.url).toContain("/api/v1/preview/lights");
    expect(socket.binaryType).toBe("arraybuffer");

    // No colours before the daemon has named the lights.
    expect(result.current("openrgb:strip-1#z0")).toBeNull();

    act(() => {
      socket.onmessage?.({ data: JSON.stringify({ "0": "openrgb:strip-1#z0", "1": "lite-b" }) } as MessageEvent);
    });
    act(() => {
      socket.onmessage?.({
        data: frame([
          { index: 0, rgb: [255, 0, 0, 0, 255, 0] },
          { index: 1, rgb: [0, 0, 255] },
        ]),
      } as MessageEvent);
    });

    expect(Array.from(result.current("openrgb:strip-1#z0") ?? [])).toEqual([255, 0, 0, 0, 255, 0]);
    expect(Array.from(result.current("lite-b") ?? [])).toEqual([0, 0, 255]);
    expect(result.current("not-a-light")).toBeNull();

    // The newest frame wins: a light's colours are its own run, never a mix of two frames.
    act(() => {
      socket.onmessage?.({
        data: frame([{ index: 0, rgb: [1, 2, 3] }]),
      } as MessageEvent);
    });
    expect(Array.from(result.current("openrgb:strip-1#z0") ?? [])).toEqual([1, 2, 3]);
    // A light the newest frame does not name keeps the run it had: a frame is not a picture of
    // the whole canvas.
    expect(Array.from(result.current("lite-b") ?? [])).toEqual([0, 0, 255]);

    unmount();
    expect(socket.closed).toBe(true);
  });

  it("forgets the colours and the index when the socket closes", async () => {
    const { result } = renderHook(() => useLightFeed());
    await waitFor(() => expect(FakeSocket.opened).toHaveLength(1));
    const socket = FakeSocket.opened[0]!;

    act(() => {
      socket.onmessage?.({ data: JSON.stringify({ "0": "lite-a", "1": "lite-b" }) } as MessageEvent);
    });
    act(() => {
      socket.onmessage?.({ data: frame([{ index: 0, rgb: [255, 0, 0] }]) } as MessageEvent);
    });
    expect(Array.from(result.current("lite-a") ?? [])).toEqual([255, 0, 0]);

    // The daemon goes away.
    vi.useFakeTimers();
    act(() => {
      socket.onclose?.({} as CloseEvent);
    });
    // The canvas shows no colours of a socket that is gone.
    expect(result.current("lite-a")).toBeNull();

    // The feed comes back.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1300);
    });
    expect(FakeSocket.opened).toHaveLength(2);
    const again = FakeSocket.opened[1]!;
    // A frame before the new index names no light: the old index is gone, so it goes nowhere.
    act(() => {
      again.onmessage?.({ data: frame([{ index: 0, rgb: [0, 255, 0] }]) } as MessageEvent);
    });
    expect(result.current("lite-a")).toBeNull();

    // The new index names the lights again, and the frames land.
    act(() => {
      again.onmessage?.({ data: JSON.stringify({ "0": "lite-a" }) } as MessageEvent);
    });
    act(() => {
      again.onmessage?.({ data: frame([{ index: 0, rgb: [0, 255, 0] }]) } as MessageEvent);
    });
    expect(Array.from(result.current("lite-a") ?? [])).toEqual([0, 255, 0]);
  });
});

// The demo draws what each group says it plays: the picture follows the effect and the mode.
describe("demoLightColours", () => {
  const scene = (id: string): Scene => structuredClone(DEMO_SCENES.scenes.find((s) => s.id === id) as Scene);
  const show = (active: Scene, failed: string[] = []) =>
    patchSnap({
      lights: structuredClone(DEMO_LIGHTS),
      scenes: {
        active: active.id,
        scenes: [active],
        group_states: active.groups.map((g) => ({ id: g.id, failed: failed.includes(g.id) ? "gone" : null })),
      },
    });
  const led = (rgb: Uint8Array | undefined, i: number) => Array.from(rgb?.slice(i * 3, i * 3 + 3) ?? []);

  it("draws White as one flat near-white over every LED", () => {
    const focus = scene("focus");
    focus.groups[0]!.brightness = 100;
    show(focus);
    const out = demoLightColours(3.2);
    const members = focus.groups[0]!.members;
    const first = led(out.get(members[0] as string), 0);
    expect(Math.min(...first)).toBeGreaterThan(220);
    for (const id of members) {
      const rgb = out.get(id);
      for (let i = 0; i < (rgb?.length ?? 0) / 3; i += 1) expect(led(rgb, i)).toEqual(first);
    }
  });

  it("flow across runs one picture over the lights; each its own repeats it on every light", () => {
    const gaming = scene("gaming");
    const ram = gaming.groups.find((g) => g.id === "g-ram")!;
    ram.mode = "flow_across";
    show(gaming);
    const flowing = demoLightColours(1);
    const a = led(flowing.get(ram.members[0] as string), 0);
    const b = led(flowing.get(ram.members[1] as string), 0);
    expect(b).not.toEqual(a);

    ram.mode = "each_its_own";
    show(gaming);
    const own = demoLightColours(1);
    expect(led(own.get(ram.members[1] as string), 0)).toEqual(led(own.get(ram.members[0] as string), 0));
  });

  it("keeps Off, a failed group and an unassigned light dark", () => {
    const evening = scene("evening");
    const lost = evening.groups.find((g) => g.id === "g-lost")!;
    show(evening, ["g-lost"]);
    const out = demoLightColours(2);
    for (const id of lost.members) expect(Array.from(out.get(id) ?? []).every((v) => v === 0)).toBe(true);
    const warm = out.get(evening.groups[0]!.members[0] as string);
    expect(Array.from(warm ?? []).some((v) => v > 0)).toBe(true);
  });
});
