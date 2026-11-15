import { describe, expect, it, vi } from "vitest";

// #339: a pattern has no canvas frame, so the daemon has no thumbnail for it. The scene's rail
// asked for one for every group playing a pattern (a failed request each time, since a failure is
// not kept). A pattern id now resolves to "no picture" without a request.
const effectThumbnail = vi.fn();
vi.mock("@/lib/endpoints", () => ({ api: { effectThumbnail } }));

describe("loadThumb for a pattern", () => {
  it("asks the daemon nothing and answers no picture", async () => {
    const { loadThumb } = await import("@/lib/thumbs");
    expect(await loadThumb("pattern:Front fans")).toBeNull();
    expect(await loadThumb("pattern:Front fans", { first: true })).toBeNull();
    expect(effectThumbnail).not.toHaveBeenCalled();
  });
});
