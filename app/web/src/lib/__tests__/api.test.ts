import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  CONFIG_TRANSFER_TIMEOUT_MS,
  DEVICE_RESCAN_TIMEOUT_MS,
  EFFECT_IMPORT_TIMEOUT_MS,
  PERIPHERAL_TIMEOUT_MS,
  REQUEST_TIMEOUT_MS,
  deadlineFor,
  dropApiConfig,
  request,
} from "@/lib/api";
import { getSnap, patchSnap, retryEffects } from "@/lib/store";
import { CMD_GET_API_CONFIG } from "@/proto/tauri";

// The deadline on a request. A request that never comes back must not leave the area pending
// for good: the frame would show a placeholder with no error and no way to try again.
//
// The deadline covers the whole request: the tray's lookup of the address, the call itself and
// the body that comes with it. A route that waits on a device gets a deadline of its own, so a
// sleeping mouse is not given up on half way through its read.

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

/** A fetch that answers, but only after the given number of milliseconds. */
function slowFetch(ms: number) {
  return vi.fn(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
    return {
      ok: true,
      status: 200,
      text: async () => JSON.stringify({ ok: true }),
      json: async () => ({ ok: true }),
    } as unknown as Response;
  });
}

/** A fetch that answers with its headers, but whose body never finishes. */
function hangingBodyFetch() {
  return vi.fn(async () => {
    return {
      ok: true,
      status: 200,
      text: () => new Promise<string>(() => {}),
      json: async () => ({}),
    } as unknown as Response;
  });
}

/** The tray: it hands out the address and the token. */
function trayThatAnswers() {
  return {
    core: {
      invoke: async (cmd: string) =>
        cmd === CMD_GET_API_CONFIG ? { base_url: "http://127.0.0.1:9430", token: "test-token" } : null,
    },
    event: { listen: async () => () => undefined },
  };
}

/** The tray: it never finishes handing out the address. */
function trayThatNeverAnswers() {
  return {
    core: {
      invoke: async (cmd: string) =>
        cmd === CMD_GET_API_CONFIG ? new Promise<never>(() => {}) : null,
    },
    event: { listen: async () => () => undefined },
  };
}

describe("the request deadline", () => {
  beforeEach(() => {
    // The tray hands the editor its address and its token.
    (window as unknown as { __TAURI__?: unknown }).__TAURI__ = trayThatAnswers();
    // The address of the last test is not the address of this one.
    dropApiConfig();
    patchSnap({ effects: null, effectsError: null, pending: [] });
  });

  afterEach(() => {
    delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
    dropApiConfig();
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

  it("gives up on a tray that never hands out the address", async () => {
    vi.useFakeTimers();
    // The tray's own lookup never finishes: the request is waiting before it even starts.
    (window as unknown as { __TAURI__?: unknown }).__TAURI__ = trayThatNeverAnswers();
    dropApiConfig();
    const call = request("GET", "/api/v1/effects");
    const settled = expect(call).rejects.toThrow(/did not answer within/);
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 50);
    await settled;
  });

  it("gives up on a body that never finishes", async () => {
    vi.useFakeTimers();
    // The headers arrive, and then nothing: the body is read under the same deadline.
    vi.stubGlobal("fetch", hangingBodyFetch());
    const call = request("GET", "/api/v1/effects");
    const settled = expect(call).rejects.toThrow(/did not answer within/);
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 50);
    await settled;
  });

  it("does not give up on a peripheral read at the default deadline", async () => {
    vi.useFakeTimers();
    // A sleeping mouse: the read takes 12.75 s at its worst (the adapter's own constants).
    vi.stubGlobal("fetch", slowFetch(12_000));
    const call = request("GET", "/api/v1/peripherals/x6");
    const settled = expect(call).resolves.toBeTruthy();
    await vi.advanceTimersByTimeAsync(12_000);
    await settled;
  });
});

describe("the deadlines of the routes", () => {
  it("pins the default deadline", () => {
    expect(REQUEST_TIMEOUT_MS).toBe(10_000);
  });

  it("pins the deadline of every route that waits on a device", () => {
    expect(PERIPHERAL_TIMEOUT_MS).toBe(30_000);
    expect(EFFECT_IMPORT_TIMEOUT_MS).toBe(60_000);
    expect(CONFIG_TRANSFER_TIMEOUT_MS).toBe(60_000);
    expect(DEVICE_RESCAN_TIMEOUT_MS).toBe(30_000);
  });

  it("gives the slow routes their own deadline and the rest the short one", () => {
    expect(deadlineFor("/api/v1/peripherals")).toBe(PERIPHERAL_TIMEOUT_MS);
    expect(deadlineFor("/api/v1/peripherals/x6")).toBe(PERIPHERAL_TIMEOUT_MS);
    expect(deadlineFor("/api/v1/peripherals/x6/access")).toBe(PERIPHERAL_TIMEOUT_MS);
    expect(deadlineFor("/api/v1/effects/import")).toBe(EFFECT_IMPORT_TIMEOUT_MS);
    expect(deadlineFor("/api/v1/config/export")).toBe(CONFIG_TRANSFER_TIMEOUT_MS);
    expect(deadlineFor("/api/v1/config/import")).toBe(CONFIG_TRANSFER_TIMEOUT_MS);
    expect(deadlineFor("/api/v1/devices?rescan=1")).toBe(DEVICE_RESCAN_TIMEOUT_MS);
    // Everything else is short: the daemon answers in well under a second.
    expect(deadlineFor("/api/v1/status")).toBe(REQUEST_TIMEOUT_MS);
    expect(deadlineFor("/api/v1/effects")).toBe(REQUEST_TIMEOUT_MS);
    expect(deadlineFor("/api/v1/devices")).toBe(REQUEST_TIMEOUT_MS);
  });
});
