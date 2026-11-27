import { describe, expect, it } from "vitest";
import { problems } from "@/lib/status";
import type { Fault } from "@/proto/Fault";
import type { Status } from "@/proto/Status";

function fault(over: Partial<Fault> = {}): Fault {
  return {
    kind: "fan_chip",
    key: "0123456789abcdef",
    affected: "Fan headers can't be driven.",
    fix: "Install PawnIO, then restart canvas-rgb.",
    dismissed: false,
    ...over,
  };
}

function status(over: Partial<Status> = {}): Status {
  return {
    active_scene: null,
    conflicts: [],
    faults: [],
    openrgb: "connected",
    service: "connected",
    power: "paused",
    parked: false,
    fps_measured: 30,
    fps_target: 30,
    devices_connected: 0,
    ...over,
  } as unknown as Status;
}

describe("hardware notes in the strip (#580 R2, R14, R15)", () => {
  it("feeds every fault into problems, including a dismissed one", () => {
    const active = fault();
    const dismissed = fault({ kind: "open_rgb_down", key: "fedcba9876543210", dismissed: true });
    const list = problems(status({ faults: [active, dismissed] }), null);
    expect(list.filter((p) => p.fault).map((p) => p.text)).toEqual([
      active.affected,
      dismissed.affected,
    ]);
    expect(list.filter((p) => p.fault).length).toBe(2);
  });

  it("drops its own OpenRGB down entry and keeps Fan control off", () => {
    const list = problems(status({ openrgb: "down", service: "down" }), null);
    expect(list.some((p) => p.text === "OpenRGB down")).toBe(false);
    expect(list.some((p) => p.text === "Fan control off")).toBe(true);
  });

  it("does not call an unchecked or connecting state a fault", () => {
    const list = problems(status({ openrgb: "connecting" }), null);
    expect(list.some((p) => p.fault)).toBe(false);
    expect(list.map((p) => p.text)).toEqual(["OpenRGB connecting"]);
  });

  it("counts two notes even when one was dismissed", () => {
    const list = problems(status({ faults: [fault(), fault({ dismissed: true, key: "1111111111111111" })] }), null);
    expect(list.filter((p) => p.fault).length).toBe(2);
  });
});

describe("an older daemon without faults (C4)", () => {
  it("keeps the old OpenRGB down entry when faults is absent", () => {
    const old = status({ openrgb: "down" });
    delete (old as { faults?: unknown }).faults;
    const list = problems(old, null);
    expect(list.some((p) => p.text === "OpenRGB down")).toBe(true);
  });

  it("drops the old entry when a fault array is present, even when empty", () => {
    const list = problems(status({ openrgb: "down", faults: [] }), null);
    expect(list.some((p) => p.text === "OpenRGB down")).toBe(false);
  });
});
