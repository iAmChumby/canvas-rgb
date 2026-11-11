import { beforeEach, describe, expect, it, vi } from "vitest";

// #296 review: a thumbnail may take up to its long deadline to answer, so the gallery must not send
// every tile at once. Six of them would hold every HTTP/1.1 socket the browser keeps for the daemon
// and stall the fan poll and Activate behind them. At most THUMBS_IN_FLIGHT are on the wire; the
// rest wait, in order, and go when a place frees (an answer or a failure).
const effectThumbnail = vi.fn();
vi.mock("@/lib/endpoints", () => ({ api: { effectThumbnail } }));

const PICTURE = new Uint8Array([1, 0, 1, 0, 1, 0, 255, 128, 0]).buffer;

beforeEach(() => {
  vi.resetModules();
  effectThumbnail.mockReset();
});

describe("loadThumb in flight", () => {
  it("sends at most THUMBS_IN_FLIGHT at once, and the first that waited goes when one lands", async () => {
    const { loadThumb, THUMBS_IN_FLIGHT, thumbsInFlight } = await import("@/lib/thumbs");
    const answers: Array<(buf: ArrayBuffer) => void> = [];
    effectThumbnail.mockImplementation(() => new Promise<ArrayBuffer>((resolve) => answers.push(resolve)));

    const tiles = Array.from({ length: 8 }, (_, i) => loadThumb(`effect-${i}`));
    await Promise.resolve();
    expect(effectThumbnail).toHaveBeenCalledTimes(THUMBS_IN_FLIGHT);
    expect(thumbsInFlight()).toBe(THUMBS_IN_FLIGHT);

    answers[0]?.(PICTURE);
    await vi.waitFor(() => expect(effectThumbnail).toHaveBeenCalledTimes(THUMBS_IN_FLIGHT + 1));
    expect(effectThumbnail.mock.calls[THUMBS_IN_FLIGHT]?.[0]).toBe(`effect-${THUMBS_IN_FLIGHT}`);
    expect(thumbsInFlight()).toBe(THUMBS_IN_FLIGHT);

    // Answer each one as it is sent: all eight go out, never more than THUMBS_IN_FLIGHT at once.
    for (let next = 1; next < 8; next++) {
      await vi.waitFor(() => expect(answers.length).toBeGreaterThan(next));
      expect(thumbsInFlight()).toBeLessThanOrEqual(THUMBS_IN_FLIGHT);
      answers[next]?.(PICTURE);
    }
    await Promise.all(tiles);
    expect(effectThumbnail).toHaveBeenCalledTimes(8);
    expect(thumbsInFlight()).toBe(0);
  });

  it("frees a place when a request fails", async () => {
    const { loadThumb, THUMBS_IN_FLIGHT } = await import("@/lib/thumbs");
    const fails: Array<(e: Error) => void> = [];
    effectThumbnail.mockImplementation(() => new Promise<ArrayBuffer>((_, reject) => fails.push(reject)));

    const tiles = Array.from({ length: THUMBS_IN_FLIGHT + 1 }, (_, i) => loadThumb(`effect-${i}`));
    await Promise.resolve();
    expect(effectThumbnail).toHaveBeenCalledTimes(THUMBS_IN_FLIGHT);
    fails[0]?.(new Error("timeout"));
    await vi.waitFor(() => expect(effectThumbnail).toHaveBeenCalledTimes(THUMBS_IN_FLIGHT + 1));
    for (const reject of fails) reject(new Error("timeout"));
    expect(await Promise.all(tiles)).toEqual(Array(THUMBS_IN_FLIGHT + 1).fill(null));
  });
});

describe("loadThumb first", () => {
  it("sends a picture the person is looking at before the tiles that waited", async () => {
    const { loadThumb, THUMBS_IN_FLIGHT } = await import("@/lib/thumbs");
    const answers: Array<(buf: ArrayBuffer) => void> = [];
    effectThumbnail.mockImplementation(() => new Promise<ArrayBuffer>((resolve) => answers.push(resolve)));
    const PICTURE = new Uint8Array([1, 0, 1, 0, 1, 0, 255, 128, 0]).buffer;

    // The gallery fills the places and the queue.
    for (let i = 0; i < THUMBS_IN_FLIGHT + 3; i++) void loadThumb(`tile-${i}`);
    // The selected effect's preview asks for a tile that already waits, and a new one.
    void loadThumb(`tile-${THUMBS_IN_FLIGHT + 2}`, { first: true });
    void loadThumb("selected", { first: true });
    await Promise.resolve();
    expect(effectThumbnail).toHaveBeenCalledTimes(THUMBS_IN_FLIGHT);

    answers[0]?.(PICTURE);
    await vi.waitFor(() => expect(effectThumbnail).toHaveBeenCalledTimes(THUMBS_IN_FLIGHT + 1));
    expect(effectThumbnail.mock.calls[THUMBS_IN_FLIGHT]?.[0]).toBe("selected");
    answers[1]?.(PICTURE);
    await vi.waitFor(() => expect(effectThumbnail).toHaveBeenCalledTimes(THUMBS_IN_FLIGHT + 2));
    expect(effectThumbnail.mock.calls[THUMBS_IN_FLIGHT + 1]?.[0]).toBe(`tile-${THUMBS_IN_FLIGHT + 2}`);
    for (const resolve of answers) resolve(PICTURE);
  });
});
