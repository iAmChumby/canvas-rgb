import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { REQUEST_TIMEOUT_MS, request } from "@/lib/api";
import { getSnap, patchSnap, retryEffects } from "@/lib/store";
import { CMD_GET_API_CONFIG } from "@/proto/tauri";

// The deadline on a request. A request that never comes back must not leave the area pending
// for good: the frame would show a placeholder with no error and no way to try again.

/** A fetch that never answers, and fails when the request is given up on. */
function hangingFetch() {
  return vi.fn(
    (_url: string, init?: RequestInit) =>
      new Promise((_resolve, reject) => {
        init?.signal?.addEventListener("abort", () => {
          reject(new DOMException("The operation was aborted.", "AbortError"));
        });
      }),
  );
}

describe("the request deadline", () => {
  beforeEach(() => {
    // The tray hands the editor its address and its token.
    (window as unknown as { __TAURI__?: unknown }).__TAURI__ = {
      core: {
        invoke: async (cmd: string) =>
          cmd === CMD_GET_API_CONFIG ? { base_url: "http://127.0.0.1:9430", token: "test-token" } : null,
      },
      event: { listen: async () => () => undefined },
    };
    patchSnap({ effects: null, effectsError: null, pending: [] });
  });

  afterEach(() => {
    delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("gives up on a request that never answers", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("fetch", hangingFetch());
    const call = request("GET", "/api/v1/effects");
    const settled = expect(call).rejects.toThrow(/did not answer within/);
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 50);
    await settled;
  });

  it("does not leave the area pending when the request never answers", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("fetch", hangingFetch());
    const retry = retryEffects();
    await vi.advanceTimersByTimeAsync(10);
    // While the request is out, the frame waits: the area is pending.
    expect(getSnap().pending).toContain("effects");
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 50);
    // The deadline passed: the area is free again and the error is there to show.
    expect(getSnap().pending).not.toContain("effects");
    expect(getSnap().effectsError).toBeTruthy();
    await retry;
  });
});
