import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dropApiConfig } from "@/lib/api";
import { CMD_GET_API_CONFIG } from "@/proto/tauri";

// Issue #296: the editor gave up on a thumbnail after REQUEST_TIMEOUT_MS (10 s), while the
// daemon renders a tile within RENDER_BUDGET (30 s, crates/canvas-daemon/src/thumbnail.rs),
// possibly after waiting in its two-slot render queue. A tile that needed 12 s showed the
// "no picture" glyph while the daemon was still working on it, and the render was wasted.
//
// The thumbnail route has its own deadline now (THUMBNAIL_TIMEOUT_MS, honoured by
// deadlineFor for `/api/v1/effects/{id}/thumbnail`), so a slow answer lands and is kept
// (one render per effect).

/** A fetch that answers with the bytes of one 1x1 one-frame tile, but only after `ms`. */
function slowBinaryFetch(ms: number) {
  return vi.fn(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
    // u16 LE width, u16 LE height, u16 LE frames, then RGB: one red pixel.
    const buf = new Uint8Array([1, 0, 1, 0, 1, 0, 255, 128, 0]).buffer;
    return {
      ok: true,
      status: 200,
      arrayBuffer: async () => buf,
    } as unknown as Response;
  });
}

describe("the thumbnail deadline", () => {
  beforeEach(() => {
    (window as unknown as { __TAURI__?: unknown }).__TAURI__ = {
      core: {
        invoke: async (cmd: string) =>
          cmd === CMD_GET_API_CONFIG ? { base_url: "http://127.0.0.1:9430", token: "test-token" } : null,
      },
      event: { listen: async () => () => undefined },
    };
    dropApiConfig();
    // The thumbs module keeps its cache in module scope, so each test gets a fresh one.
    vi.resetModules();
  });

  afterEach(() => {
    delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
    dropApiConfig();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("lands a tile whose answer takes 12 s, and keeps it", async () => {
    vi.useFakeTimers();
    const fetchMock = slowBinaryFetch(12_000);
    vi.stubGlobal("fetch", fetchMock);
    const { loadThumb } = await import("@/lib/thumbs");

    const first = loadThumb("wet-smear");
    await vi.advanceTimersByTimeAsync(12_000);
    const thumb = await first;
    expect(thumb).not.toBeNull();
    expect(thumb?.w).toBe(1);
    expect(fetchMock).toHaveBeenCalledTimes(1);

    // The picture is kept: a tile that mounts again does not ask the daemon once more.
    expect((await loadThumb("wet-smear"))?.w).toBe(1);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
