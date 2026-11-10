import { describe, expect, it, vi } from "vitest";

// #258 review: a failed thumbnail fetch was kept in the cache for the whole session, so a daemon
// hiccup showed "no picture" until the page was reloaded. A failure is now not kept: the next tile
// that mounts asks again. A picture that arrived is still kept (one render per effect).
const effectThumbnail = vi.fn();
vi.mock("@/lib/endpoints", () => ({ api: { effectThumbnail } }));

describe("loadThumb", () => {
  it("asks again after a failed load, and keeps a picture once one arrives", async () => {
    const { loadThumb } = await import("@/lib/thumbs");
    effectThumbnail.mockRejectedValueOnce(new Error("daemon busy"));
    expect(await loadThumb("aurora")).toBeNull();
    expect(effectThumbnail).toHaveBeenCalledTimes(1);

    // The daemon answers now; the tile that mounts next fetches again instead of reading the miss.
    // One frame of one pixel: u16 width, u16 height, u16 frames, then RGB.
    const buf = new Uint8Array([1, 0, 1, 0, 1, 0, 255, 128, 0]).buffer;
    effectThumbnail.mockResolvedValueOnce(buf);
    expect((await loadThumb("aurora"))?.w).toBe(1);
    expect(effectThumbnail).toHaveBeenCalledTimes(2);

    // A picture that arrived is kept: a third tile does not ask a third time.
    await loadThumb("aurora");
    expect(effectThumbnail).toHaveBeenCalledTimes(2);
  });
});
