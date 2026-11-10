import { describe, expect, it } from "vitest";
import { isUnread, readAgo, STALE_AFTER_MS } from "@/lib/peripherals";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

// Issue #127: the words for when the values were read, and when a device counts as not read yet.

const NOW = 1_800_000_000_000;

describe("readAgo", () => {
  it("says just now under a minute, then minutes, hours and days", () => {
    expect(readAgo(NOW - 5_000, NOW).text).toBe("Read just now");
    expect(readAgo(NOW - 60_000, NOW).text).toBe("Read 1 min ago");
    expect(readAgo(NOW - 3 * 60_000 - 20_000, NOW).text).toBe("Read 3 min ago");
    expect(readAgo(NOW - 59 * 60_000, NOW).text).toBe("Read 59 min ago");
    expect(readAgo(NOW - 2 * 3_600_000, NOW).text).toBe("Read 2 h ago");
    expect(readAgo(NOW - 3 * 86_400_000, NOW).text).toBe("Read 3 d ago");
  });

  it("is stale only past thirty minutes", () => {
    expect(readAgo(NOW - STALE_AFTER_MS, NOW).stale).toBe(false);
    expect(readAgo(NOW - STALE_AFTER_MS - 1, NOW).stale).toBe(true);
    expect(readAgo(NOW - 60_000, NOW).stale).toBe(false);
  });

  it("never reads from the future when the clocks disagree", () => {
    expect(readAgo(NOW + 90_000, NOW)).toEqual({ text: "Read just now", stale: false });
  });
});

describe("isUnread", () => {
  const base = {
    access: { reads: true, writes: false },
    values: {},
  } as unknown as PeripheralDetail;

  it("is true with reads on, no read time and no values", () => {
    expect(isUnread(base)).toBe(true);
  });

  it("is false once there is a read time or any value, or while reads are off", () => {
    expect(isUnread({ ...base, read_at_ms: 1 as unknown as bigint })).toBe(false);
    expect(isUnread({ ...base, values: { a: { type: "bool", value: true } } })).toBe(false);
    expect(isUnread({ ...base, access: { reads: false, writes: false } })).toBe(false);
  });
});
