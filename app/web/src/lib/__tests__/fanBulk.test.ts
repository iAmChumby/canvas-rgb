import { describe, expect, it, vi } from "vitest";
import { TACH_ONLY_WHY, UNREAD_DUTY_WHY, applyBulk, canDrive, isUnavailable } from "@/lib/fanBulk";
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

  it("names a header with an unread duty as unread, and a daemon-labelled tach-only header as tach only (#279)", async () => {
    const unread = { ...header("cpu_fan"), pwm_percent: null } as FanHeaderStatus;
    const tach = { ...header("pump_fan2"), label: "Pump fan 2 (tach only)", pwm_percent: null } as FanHeaderStatus;
    const fans = { headers: [unread, tach], floor_percent: 20 } as unknown as FansStatus;
    const write = vi.fn(async () => "sent" as const);
    const out = await applyBulk(fans.headers, { kind: "fixed", percent: 50 }, fans, write);
    expect(write).not.toHaveBeenCalled();
    expect(out.find((o) => o.id === "cpu_fan")?.detail).toBe(UNREAD_DUTY_WHY);
    expect(out.find((o) => o.id === "pump_fan2")?.detail).toBe(TACH_ONLY_WHY);
  });

  it("still sends BIOS to a header with no duty reading: it is the way back from Fixed or Curve (#279)", async () => {
    const stuck = { ...header("cpu_fan"), pwm_percent: null, mode: { mode: "fixed", percent: 60 }, on_bios: false } as FanHeaderStatus;
    const fans = { headers: [stuck], floor_percent: 20 } as unknown as FansStatus;
    const write = vi.fn(async () => "sent" as const);
    const out = await applyBulk(fans.headers, { kind: "bios" }, fans, write);
    expect(write).toHaveBeenCalledWith("cpu_fan", { mode: "bios" }, expect.any(Function));
    expect(out.find((o) => o.id === "cpu_fan")?.state).toBe("applied");
  });
});
