import { act, cleanup, renderHook } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The per-group preview (#378): one socket for the group the stage shows. It is closed when
// the stage goes, when the window is hidden and when another group takes its place, and the
// old socket is always closed before the new one is made.

const api = vi.hoisted(() => ({
  apiConfig: vi.fn(),
  dropApiConfig: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, apiConfig: api.apiConfig, dropApiConfig: api.dropApiConfig };
});

import { useGroupPreview } from "@/views/scenes/useGroupPreview";
import type { ApiConfig } from "@/proto/ApiConfig";

const CONFIG: ApiConfig = { base_url: "http://127.0.0.1:9430", token: "tok" };

class FakeSocket {
  static opened: FakeSocket[] = [];
  /** Was the socket before this one closed when this one was made? */
  static closedBefore: boolean[] = [];
  binaryType = "";
  onopen: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor(readonly url: string) {
    const previous = FakeSocket.opened[FakeSocket.opened.length - 1];
    FakeSocket.closedBefore.push(previous ? previous.closed : true);
    FakeSocket.opened.push(this);
  }
  close(): void {
    this.closed = true;
  }
}

/** One frame in the daemon's format: u16 LE width, u16 LE height, then the RGB bytes. */
function frame(w = 80, h = 50): ArrayBuffer {
  const buf = new ArrayBuffer(4 + w * h * 3);
  const view = new DataView(buf);
  view.setUint16(0, w, true);
  view.setUint16(2, h, true);
  return buf;
}

/** Lets the connect path finish: the token arrives and the socket is made. */
async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });
}

/** The window is shown or hidden, the way the browser does it. */
function visibility(state: "visible" | "hidden"): void {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

beforeEach(() => {
  FakeSocket.opened = [];
  FakeSocket.closedBefore = [];
  api.dropApiConfig.mockReset();
  api.apiConfig.mockReset();
  api.apiConfig.mockResolvedValue(CONFIG);
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeSocket as unknown as typeof WebSocket;
  window.__TAURI__ = { core: { invoke: async () => CONFIG } } as unknown as never;
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  delete window.__TAURI__;
});

describe("the per-group preview", () => {
  it("opens one socket for the group, on the route from the spine", async () => {
    renderHook(() => useGroupPreview({ groupId: "g one/1", saved: true }));
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);

    const url = new URL(FakeSocket.opened[0]!.url);
    expect(url.pathname).toBe("/api/v1/preview/group");
    // The group id and the token are encoded: a group id may hold anything.
    expect(url.searchParams.get("group")).toBe("g one/1");
    expect(url.searchParams.get("token")).toBe("tok");
    expect(FakeSocket.opened[0]!.binaryType).toBe("arraybuffer");
  });

  it("hands every picture to the painter", async () => {
    const feed = renderHook(() => useGroupPreview({ groupId: "g-one", saved: true }));
    await settle();
    const pictures: number[] = [];
    feed.result.current.onFrame((f) => pictures.push(f.width));

    act(() => {
      FakeSocket.opened[0]!.onmessage?.({ data: frame() } as MessageEvent);
    });
    expect(pictures).toEqual([80]);
    expect(feed.result.current.state).toBe("live");
  });

  it("opens no socket for a group the daemon has not saved, and opens one when the save lands", async () => {
    const feed = renderHook(
      ({ saved }: { saved: boolean }) => useGroupPreview({ groupId: "g-one", saved }),
      { initialProps: { saved: false } },
    );
    await settle();
    // A draft group is answered with 404: the stage waits for the save instead.
    expect(FakeSocket.opened).toHaveLength(0);
    expect(feed.result.current.state).toBe("connecting");

    feed.rerender({ saved: true });
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);
  });

  it("waits for the save when the socket is refused before it opened, and never says it stopped", async () => {
    const feed = renderHook(
      ({ saved }: { saved: boolean }) => useGroupPreview({ groupId: "g-one", saved }),
      { initialProps: { saved: true } },
    );
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);

    // The daemon refuses it: the group is not in its scene yet.
    act(() => {
      FakeSocket.opened[0]!.onclose?.({ code: 1006 } as CloseEvent);
    });
    // The save has not landed: the stage waits for it.
    feed.rerender({ saved: false });
    expect(feed.result.current.state).toBe("connecting");

    // The save lands: the socket is opened again.
    feed.rerender({ saved: true });
    await settle();
    expect(FakeSocket.opened).toHaveLength(2);
    expect(feed.result.current.state).not.toBe("stopped");
  });

  it("does not try again when another group preview took over", async () => {
    const feed = renderHook(() => useGroupPreview({ groupId: "g-one", saved: true }));
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);

    act(() => {
      FakeSocket.opened[0]!.onclose?.({ code: 1013 } as CloseEvent);
    });
    expect(feed.result.current.state).toBe("stopped");

    // No retry: the socket stays closed and the stage waits for the owner.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20000);
    });
    expect(FakeSocket.opened).toHaveLength(1);
    expect(feed.result.current.state).toBe("stopped");
  });

  it("tries a closed socket again, and gives up with the preview stopped", async () => {
    const feed = renderHook(() => useGroupPreview({ groupId: "g-one", saved: true }));
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);

    // The first close: the socket is tried again after the light feed's wait.
    act(() => {
      FakeSocket.opened[0]!.onclose?.({ code: 1006 } as CloseEvent);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1300);
    });
    expect(FakeSocket.opened).toHaveLength(2);

    act(() => {
      FakeSocket.opened[1]!.onclose?.({ code: 1006 } as CloseEvent);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2500);
    });
    expect(FakeSocket.opened).toHaveLength(3);

    act(() => {
      FakeSocket.opened[2]!.onclose?.({ code: 1006 } as CloseEvent);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(4900);
    });
    expect(FakeSocket.opened).toHaveLength(4);

    // The tries are spent: the stage says the preview stopped.
    act(() => {
      FakeSocket.opened[3]!.onclose?.({ code: 1006 } as CloseEvent);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20000);
    });
    expect(feed.result.current.state).toBe("stopped");
    expect(FakeSocket.opened).toHaveLength(4);
  });

  it("closes the first socket before the second one is made when the group changes", async () => {
    const feed = renderHook(
      ({ groupId }: { groupId: string }) => useGroupPreview({ groupId, saved: true }),
      { initialProps: { groupId: "g-one" } },
    );
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);
    const first = FakeSocket.opened[0]!;

    feed.rerender({ groupId: "g-two" });
    // The old socket is gone before the new one is made.
    expect(first.closed).toBe(true);
    await settle();
    expect(FakeSocket.opened).toHaveLength(2);
    expect(FakeSocket.closedBefore[1]).toBe(true);
    expect(new URL(FakeSocket.opened[1]!.url).searchParams.get("group")).toBe("g-two");
  });

  it("closes the socket when the stage goes", async () => {
    const feed = renderHook(() => useGroupPreview({ groupId: "g-one", saved: true }));
    await settle();
    const socket = FakeSocket.opened[0]!;
    feed.unmount();
    expect(socket.closed).toBe(true);
  });

  it("closes the socket when the window is hidden, and opens it again when it is shown", async () => {
    renderHook(() => useGroupPreview({ groupId: "g-one", saved: true }));
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);

    visibility("hidden");
    expect(FakeSocket.opened[0]!.closed).toBe(true);

    visibility("visible");
    await settle();
    expect(FakeSocket.opened).toHaveLength(2);
  });

  it("leaves exactly one socket under strict mode", async () => {
    function Probe() {
      useGroupPreview({ groupId: "g-one", saved: true });
      return null;
    }
    renderHook(() => (
      <StrictMode>
        <Probe />
      </StrictMode>
    ));
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);
  });
});
