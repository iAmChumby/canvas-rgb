import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dropApiConfig } from "@/lib/api";
import type { TauriGlobal } from "@/lib/host";
import { startPreview, stopPreview } from "@/lib/preview";

// The canvas preview feed against a fake host and a fake socket: the socket is opened only
// after the token arrives, so a feed that was stopped in that window must open no socket at
// all (the daemon would hold a preview slot for a feed nobody looks at).

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

/** The token request the test holds until it says so. */
function heldToken() {
  let asked = 0;
  let answer!: () => void;
  const promise = new Promise<{ base_url: string; token: string }>((resolve) => {
    answer = () => resolve({ base_url: "http://127.0.0.1:9430", token: "tok" });
  });
  return {
    promise,
    answer,
    get asked() {
      return asked;
    },
    ask: () => {
      asked += 1;
      return promise;
    },
  };
}

/** Lets every promise the connect path awaits settle, including the config lookup. */
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await Promise.resolve();
}

beforeEach(() => {
  FakeSocket.opened = [];
  vi.useFakeTimers();
  // The token is asked for again in every test: the answer is cached by the client.
  dropApiConfig();
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeSocket as unknown as typeof WebSocket;
  window.__TAURI__ = {
    core: { invoke: async () => ({ base_url: "http://127.0.0.1:9430", token: "tok" }) },
    event: { listen: async () => () => undefined },
  } as unknown as TauriGlobal;
});

afterEach(() => {
  stopPreview();
  vi.useRealTimers();
  delete window.__TAURI__;
});

describe("the preview feed", () => {
  it("opens no socket when the feed is stopped while the token request is pending", async () => {
    const token = heldToken();
    window.__TAURI__ = {
      core: { invoke: () => token.ask() },
      event: { listen: async () => () => undefined },
    } as unknown as TauriGlobal;
    startPreview();
    await settle();
    expect(token.asked).toBe(1);
    expect(FakeSocket.opened).toHaveLength(0);

    // The feed is stopped before the token arrives: no socket is opened for it.
    stopPreview();
    token.answer();
    await settle();
    expect(FakeSocket.opened).toHaveLength(0);
  });

  it("opens one socket for the token it asked for", async () => {
    const token = heldToken();
    window.__TAURI__ = {
      core: { invoke: () => token.ask() },
      event: { listen: async () => () => undefined },
    } as unknown as TauriGlobal;
    startPreview();
    await settle();
    // Starting the feed again while it waits does not ask the tray a second time.
    startPreview();
    await settle();
    expect(token.asked).toBe(1);

    token.answer();
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);
  });
});
