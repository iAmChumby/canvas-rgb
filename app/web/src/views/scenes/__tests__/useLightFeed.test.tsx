import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The per-light feed (#379): one socket for the whole window, opened by the first holder and
// closed by the last. The socket is opened only after the token arrives, so these tests hold
// the token request and watch what the feed does while it waits.
//
// The token request is mocked so that every call gets a request of its own: that is the worst
// case (the tray's answer was dropped between two calls), and it is the case where a second
// connect in the window between the request and the socket would open a second socket.

const api = vi.hoisted(() => ({
  apiConfig: vi.fn(),
  dropApiConfig: vi.fn(),
}));
vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, apiConfig: api.apiConfig, dropApiConfig: api.dropApiConfig };
});

import { useLightFeed } from "@/views/scenes/useLightFeed";
import type { ApiConfig } from "@/proto/ApiConfig";

const CONFIG: ApiConfig = { base_url: "http://127.0.0.1:9430", token: "tok" };

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

/** A token request the test holds until it says so. */
interface TokenRequest {
  resolve: () => void;
  reject: () => void;
}

let requests: TokenRequest[] = [];

beforeEach(() => {
  FakeSocket.opened = [];
  requests = [];
  api.dropApiConfig.mockClear();
  api.apiConfig.mockReset();
  api.apiConfig.mockImplementation(
    () =>
      new Promise<ApiConfig>((resolve, reject) => {
        requests.push({
          resolve: () => resolve(CONFIG),
          reject: () => reject(new Error("the tray did not answer")),
        });
      }),
  );
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeSocket as unknown as typeof WebSocket;
  window.__TAURI__ = { core: { invoke: async () => CONFIG } } as unknown as never;
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  delete window.__TAURI__;
});

/** Answers the token request the feed is waiting on. */
async function answer(request: TokenRequest): Promise<void> {
  await act(async () => {
    request.resolve();
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

describe("the per-light feed", () => {
  it("asks the tray for the token once, however many times the window is shown and hidden while it waits", async () => {
    const feed = renderHook(() => useLightFeed());
    expect(requests).toHaveLength(1);

    // The window is hidden and shown again while the token request is still pending.
    visibility("hidden");
    visibility("visible");
    expect(requests).toHaveLength(1);

    await answer(requests[0]!);
    expect(FakeSocket.opened).toHaveLength(1);

    // The last holder goes: the socket goes with it.
    feed.unmount();
    expect(FakeSocket.opened[0]!.closed).toBe(true);
  });

  it("opens one socket again when the window is shown after being hidden", async () => {
    const feed = renderHook(() => useLightFeed());
    await answer(requests[0]!);
    expect(FakeSocket.opened).toHaveLength(1);

    visibility("hidden");
    expect(FakeSocket.opened[0]!.closed).toBe(true);

    visibility("visible");
    expect(requests).toHaveLength(2);
    await answer(requests[1]!);
    expect(FakeSocket.opened).toHaveLength(2);

    feed.unmount();
    expect(FakeSocket.opened[1]!.closed).toBe(true);
  });

  it("reconnects one socket after the server closes it", async () => {
    const feed = renderHook(() => useLightFeed());
    await answer(requests[0]!);
    expect(FakeSocket.opened).toHaveLength(1);

    // The server closes the socket: the feed waits, then tries again.
    act(() => {
      FakeSocket.opened[0]!.onclose?.({} as CloseEvent);
    });
    expect(FakeSocket.opened).toHaveLength(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(requests).toHaveLength(2);
    await answer(requests[1]!);
    expect(FakeSocket.opened).toHaveLength(2);

    feed.unmount();
    expect(FakeSocket.opened[1]!.closed).toBe(true);
  });

  it("leaves no socket when the token request fails, and asks again", async () => {
    const feed = renderHook(() => useLightFeed());
    expect(requests).toHaveLength(1);

    await act(async () => {
      requests[0]!.reject();
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(FakeSocket.opened).toHaveLength(0);

    // The feed waits, then asks the tray again.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(requests).toHaveLength(2);
    await answer(requests[1]!);
    expect(FakeSocket.opened).toHaveLength(1);

    feed.unmount();
  });

  it("opens no socket when the last holder goes while the token request is pending", async () => {
    const feed = renderHook(() => useLightFeed());
    expect(requests).toHaveLength(1);

    feed.unmount();
    await answer(requests[0]!);
    expect(FakeSocket.opened).toHaveLength(0);
  });
});
