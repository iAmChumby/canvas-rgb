import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onStackPreviewFeed, startStackPreview, stopStackPreview } from "@/lib/stackPreview";
import type { TauriGlobal } from "@/lib/host";

// The builder's preview feed against a fake host and a fake socket: the daemon opens one stack
// preview at a time, and a second one is refused with code 1013 and a reason (PR #100).

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

/** Lets every promise the connect path awaits settle, including the config lookup. */
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await Promise.resolve();
}

beforeEach(() => {
  FakeSocket.opened = [];
  vi.useFakeTimers();
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeSocket as unknown as typeof WebSocket;
  window.__TAURI__ = {
    core: { invoke: async () => ({ base_url: "http://127.0.0.1:9430", token: "tok" }) },
    event: { listen: async () => () => undefined },
  } as unknown as TauriGlobal;
});

afterEach(() => {
  stopStackPreview();
  vi.useRealTimers();
  delete window.__TAURI__;
});

describe("the stack preview feed", () => {
  it("reports the daemon's reason when a second stack preview is refused", async () => {
    const problems: Array<string | null> = [];
    const off = onStackPreviewFeed((problem) => problems.push(problem));
    try {
      startStackPreview();
      await settle();
      expect(FakeSocket.opened).toHaveLength(1);
      const socket = FakeSocket.opened[0]!;
      expect(socket.url).toContain("stack=1");

      socket.onclose?.({ code: 1013, reason: "another stack preview is open; close it first" } as CloseEvent);
      expect(problems).toContain("another stack preview is open; close it first");

      // The refused socket is asked again, but at a walking pace: not within the usual 1.2 s.
      await settle();
      vi.advanceTimersByTime(1_300);
      await settle();
      expect(FakeSocket.opened).toHaveLength(1);
      vi.advanceTimersByTime(3_700);
      await settle();
      expect(FakeSocket.opened).toHaveLength(2);

      // A socket that opens clears the reason.
      FakeSocket.opened[1]!.onopen?.();
      expect(problems.at(-1)).toBeNull();
    } finally {
      off();
    }
  });

  it("keeps retrying a socket that closes for any other reason, and says nothing", async () => {
    const problems: Array<string | null> = [];
    const off = onStackPreviewFeed((problem) => problems.push(problem));
    try {
      startStackPreview();
      await settle();
      FakeSocket.opened[0]!.onclose?.({ code: 1006, reason: "" } as CloseEvent);
      await settle();
      vi.advanceTimersByTime(1_300);
      await settle();
      expect(FakeSocket.opened).toHaveLength(2);
      expect(problems).toEqual([]);
    } finally {
      off();
    }
  });
});
