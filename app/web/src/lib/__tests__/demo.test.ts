import { describe, expect, it } from "vitest";
import { checkPeripheralValue, demoRequest } from "@/lib/demo";
import type { DemoPeripheral } from "@/lib/peripheralFixtures";
import { ApiFailure, messageOf } from "@/lib/errors";

// The demo backend's own words. The editor shows them behind a disclosure, and they are the
// only words a person reads when nothing answers: they say canvas-rgb, not "the daemon".

describe("the demo backend's words", () => {
  it("says canvas-rgb, not the daemon, when nothing answers", async () => {
    window.history.replaceState({}, "", "/?state=daemon-down");
    let message = "";
    try {
      await demoRequest("GET", "/api/v1/status", undefined);
    } catch (e) {
      message = messageOf(e);
    }
    expect(message).toMatch(/canvas-rgb/);
    expect(message).not.toMatch(/daemon/i);
  });
});

describe("the demo's header availability answer (#146)", () => {
  it("answers an unknown header with 422 validation, like the daemon", async () => {
    window.history.replaceState({}, "", "/");
    const err = await demoRequest("PUT", "/api/v1/fans/headers/not_a_header/availability", { available: false }).then(
      () => null,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiFailure);
    expect((err as ApiFailure).status).toBe(422);
    expect((err as ApiFailure).code).toBe("validation");
  });

  it("refuses a body without available, like the real API", async () => {
    window.history.replaceState({}, "", "/");
    const err = await demoRequest("PUT", "/api/v1/fans/headers/sys_fan1/availability", {}).then(
      () => null,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiFailure);
    expect((err as ApiFailure).status).toBe(422);
    expect((err as ApiFailure).code).toBe("validation");
  });
});

describe("the demo's value validator (#106)", () => {
  const device = {
    info: { id: "test:1", name: "Test", vendor: "", kind: "mouse", adapter: "test", serial: "", location: "" },
    schema: {
      device: "test:1",
      kind: "mouse",
      read_only: [],
      unreported: [],
      sections: [
        {
          id: "lighting",
          title: "Lighting",
          properties: [
            { key: "angle", label: "Angle", kind: { type: "angle", default: 90 } },
            { key: "tint", label: "Tint", kind: { type: "hue", default: 0 } },
          ],
        },
      ],
    },
    values: {},
    battery: null,
    charging: false,
  } as unknown as DemoPeripheral;

  it("takes an angle value for an angle setting", () => {
    expect(() => checkPeripheralValue(device, "angle", { type: "angle", value: 120 })).not.toThrow();
  });

  it("takes a saved hue for an angle setting, as the same degrees", () => {
    expect(() => checkPeripheralValue(device, "angle", { type: "hue", value: 120 })).not.toThrow();
  });

  it("takes a saved number for an angle setting, as the same degrees (#121)", () => {
    expect(() => checkPeripheralValue(device, "angle", { type: "number", value: 45 })).not.toThrow();
  });

  it("still refuses the kinds that do not fit", () => {
    expect(() => checkPeripheralValue(device, "angle", { type: "number", value: NaN })).toThrow();
    expect(() => checkPeripheralValue(device, "tint", { type: "angle", value: 120 })).toThrow();
    expect(() => checkPeripheralValue(device, "tint", { type: "number", value: 120 })).toThrow();
  });
});
