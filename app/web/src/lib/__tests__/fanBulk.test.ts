import { describe, expect, it, vi } from "vitest";
import { applyBulk, canDrive, isUnavailable } from "@/lib/fanBulk";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";
import type { FansStatus } from "@/proto/FansStatus";

// Issue #128: a header the board does not route is never ticked and never sent in bulk.

const header = (id: string, available = true): FanHeaderStatus =>
  ({
    id,
    label: id,
    rpm: 900,
    pwm_percent: 30,
    mode: { mode: "bios" },
    on_bios: true,
    override_reason: null,
    available,
  }) as FanHeaderStatus;

describe("canDrive", () => {
  it("is false for a header that is not on this board", () => {
    expect(canDrive(header("sys_fan5", false))).toBe(false);
    expect(canDrive(header("sys_fan1"))).toBe(true);
  });

  it("isUnavailable only for unavailable headers", () => {
    expect(isUnavailable(header("sys_fan5", false))).toBe(true);
    expect(isUnavailable(header("sys_fan1"))).toBe(false);
  });
});

describe("applyBulk", () => {
  it("never sends an unavailable header and reports it skipped", async () => {
    const fans = {
      headers: [header("sys_fan1"), header("sys_fan5", false)],
      floor_percent: 20,
    } as unknown as FansStatus;
    const write = vi.fn(async () => "sent" as const);
    const out = await applyBulk(fans.headers, { kind: "fixed", percent: 50 }, fans, write);
    expect(write).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledWith("sys_fan1", { mode: "fixed", percent: 50 }, expect.any(Function));
    const unavailable = out.find((o) => o.id === "sys_fan5");
    expect(unavailable?.state).toBe("skipped");
    expect(unavailable?.detail).toMatch(/Not on this board/);
    expect(out.find((o) => o.id === "sys_fan1")?.state).toBe("applied");
  });
});
