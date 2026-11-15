import { describe, expect, it, vi } from "vitest";
import { checkPeripheralValue, demoRequest } from "@/lib/demo";
import type { DemoPeripheral } from "@/lib/peripheralFixtures";
import { ApiFailure, messageOf } from "@/lib/errors";
import type { EffectsResponse } from "@/proto/EffectsResponse";
import type { Pattern } from "@/proto/Pattern";
import type { PatternsResponse } from "@/proto/PatternsResponse";

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

describe("the demo's patterns (#339)", () => {
  /**
   * A fresh demo backend with `?state=patterns`: the demo keeps one state per page, and this file's
   * earlier tests have already opened theirs. `vi.resetModules` gives this test the page a browser
   * opens on that address.
   */
  async function patternsDemo(): Promise<typeof import("@/lib/demo")> {
    vi.resetModules();
    window.history.replaceState({}, "", "/effects/?state=patterns");
    return import("@/lib/demo");
  }

  it("demo mode serves two patterns and a library with one", async () => {
    const demo = await patternsDemo();
    const list = (await demo.demoRequest("GET", "/api/v1/patterns", undefined)) as PatternsResponse;
    expect(list.patterns).toHaveLength(2);
    const ring = list.patterns.find((p) => p.id === "pattern:Intake");
    const strip = list.patterns.find((p) => p.id === "pattern:Underglow");
    expect(ring?.built_on).toEqual({ led_count: 12, arrangement: "ring" });
    expect(strip?.built_on).toEqual({ led_count: 30, arrangement: "strip" });
    // A ring of 12 with one marked LED, a strip of 30 with three.
    expect(ring?.frames[0]?.colors).toHaveLength(12);
    expect(ring?.frames[0]?.colors.filter((c) => c !== "#000000")).toHaveLength(1);
    expect(strip?.frames[0]?.colors).toHaveLength(30);
    expect(strip?.frames[0]?.colors.filter((c) => c !== "#000000")).toHaveLength(3);

    // The library lists both as patterns, each with what it was built on, beside the canvas ones.
    const effects = (await demo.demoRequest("GET", "/api/v1/effects", undefined)) as EffectsResponse;
    const patterns = effects.effects.filter((e) => e.kind === "pattern");
    expect(patterns.map((e) => e.id).sort()).toEqual(["pattern:Intake", "pattern:Underglow"]);
    expect(patterns.find((e) => e.id === "pattern:Intake")?.built_on).toEqual({
      led_count: 12,
      arrangement: "ring",
    });
    expect(patterns.find((e) => e.id === "pattern:Intake")?.description).toBe("Pattern, ring of 12");
    expect(patterns.find((e) => e.id === "pattern:Intake")?.needs).toEqual({ screen: false, audio: false });
    expect(effects.effects.find((e) => e.id === "aurora")?.kind).toBe("canvas");

    // One pattern by its id, and by its bare name.
    const one = (await demo.demoRequest("GET", "/api/v1/patterns/pattern%3AIntake", undefined)) as Pattern;
    expect(one.id).toBe("pattern:Intake");
    const byName = (await demo.demoRequest("GET", "/api/v1/patterns/Intake", undefined)) as Pattern;
    expect(byName.id).toBe("pattern:Intake");
  });

  it("putPattern sends the whole document and returns the saved one", async () => {
    await patternsDemo();
    const { api } = await import("@/lib/endpoints");
    // A document with what this version does not know: a motion other than none, an unknown key on
    // the pattern and one on its frame.
    const document = {
      id: "pattern:Intake",
      built_on: { led_count: 3, arrangement: "ring" },
      frames: [{ colors: ["#ff0000", "#000000", "#000000"], hold: 0, label: "the one that plays" }],
      motion: { type: "rotate", speed: 2 },
      tags: ["front"],
    } as unknown as Pattern;

    const saved = await api.putPattern("pattern:Intake", document);

    expect(saved.id).toBe("pattern:Intake");
    expect(saved.frames[0]?.colors).toEqual(["#ff0000", "#000000", "#000000"]);
    expect(saved).toMatchObject({ motion: { type: "rotate", speed: 2 }, tags: ["front"] });
    expect((saved.frames[0] as unknown as { label: string }).label).toBe("the one that plays");
    // What was saved is what the next read answers, whole.
    expect(await api.pattern("pattern:Intake")).toEqual(saved);

    // A name another pattern has, ignoring case, is a 409, as the daemon's is.
    const taken = await api
      .putPattern("pattern:intake", { ...document, id: "pattern:intake" })
      .then(() => null, (e: unknown) => e);
    expect((taken as ApiFailure).status).toBe(409);

    // A frame that is not the length the pattern was built on is a 422, with the daemon's sentence.
    const short = await api
      .putPattern("pattern:Short", {
        id: "pattern:Short",
        built_on: { led_count: 4, arrangement: "ring" },
        frames: [{ colors: ["#ff0000"], hold: 0 }],
        motion: { type: "none" },
      })
      .then(() => null, (e: unknown) => e);
    expect((short as ApiFailure).status).toBe(422);
    expect(messageOf(short)).toMatch(/frame 1 has 1 colours; the device it was built on has 4 LEDs/);
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

describe("the demo's white balance (#50)", () => {
  const deviceId = "openrgb:strip-1";
  const put = (gain: number) =>
    demoRequest("PUT", `/api/v1/devices/${encodeURIComponent(deviceId)}/white-balance`, {
      red: gain,
      green: 1,
      blue: 1,
    });

  it("answers an empty map, then PUT/GET/DELETE round-trip one device", async () => {
    window.history.replaceState({}, "", "/devices/");
    const empty = await demoRequest("GET", "/api/v1/white-balance", undefined) as { devices: unknown };
    expect(empty.devices).toEqual({});
    const putRes = await put(0.8) as { devices: Record<string, unknown> };
    expect(putRes.devices[deviceId]).toEqual({ red: 0.8, green: 1, blue: 1 });
    const got = await demoRequest("GET", "/api/v1/white-balance", undefined) as { devices: Record<string, unknown> };
    expect(got.devices[deviceId]).toEqual({ red: 0.8, green: 1, blue: 1 });
    const del = await demoRequest(
      "DELETE",
      `/api/v1/devices/${encodeURIComponent(deviceId)}/white-balance`,
      undefined,
    ) as { devices: Record<string, unknown> };
    expect(del.devices[deviceId]).toBeUndefined();
    expect(await demoRequest("GET", "/api/v1/white-balance", undefined)).toEqual({ devices: {} });
  });

  it("refuses a gain outside 0..1 with 422 validation, like the daemon", async () => {
    window.history.replaceState({}, "", "/devices/");
    const err = await put(1.5).then(
      () => null,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiFailure);
    expect((err as ApiFailure).status).toBe(422);
    expect((err as ApiFailure).code).toBe("validation");
  });

  it("answers as the daemon: NaN refused, neutral stored as nothing, an unknown device kept", async () => {
    window.history.replaceState({}, "", "/devices/");
    const nan = await put(Number.NaN).then(() => null, (e: unknown) => e);
    expect((nan as ApiFailure).status).toBe(422);
    await put(0.5);
    expect(await put(1)).toEqual({ devices: {} });
    const other = await demoRequest("PUT", "/api/v1/devices/openrgb%3Anot-here/white-balance", {
      red: 0.5,
      green: 1,
      blue: 1,
    }) as { devices: Record<string, unknown> };
    expect(other.devices["openrgb:not-here"]).toEqual({ red: 0.5, green: 1, blue: 1 });
    await demoRequest("DELETE", "/api/v1/devices/openrgb%3Anot-here/white-balance", undefined);
  });
});

describe("the demo's settings PUT (#50)", () => {
  it("keeps the stored warmth when the body has none, and refuses one out of range", async () => {
    window.history.replaceState({}, "", "/settings/");
    const start = await demoRequest("GET", "/api/v1/settings", undefined) as Record<string, unknown>;
    await demoRequest("PUT", "/api/v1/settings", { ...start, warmth: 2700 });
    const old: Record<string, unknown> = { ...start };
    delete old.warmth;
    const kept = await demoRequest("PUT", "/api/v1/settings", old) as { warmth: number };
    expect(kept.warmth).toBe(2700);
    const err = await demoRequest("PUT", "/api/v1/settings", { ...start, warmth: 100 }).then(() => null, (e: unknown) => e);
    expect((err as ApiFailure).status).toBe(422);
    await demoRequest("PUT", "/api/v1/settings", start);
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

describe("demo chain setup", () => {
  it("a commit leaves the header's zone at the parts' length in the device list", async () => {
    const { chains } = (await demoRequest("GET", "/api/v1/chains", undefined)) as {
      chains: { device_id: string; zone_index: number; zone_name: string }[];
    };
    const chain = chains.find((c) => c.zone_name === "JARGB 1");
    expect(chain).toBeDefined();
    if (!chain) return;
    const setup = `/api/v1/chains/${encodeURIComponent(chain.device_id)}/${chain.zone_index}/setup`;
    const { session } = (await demoRequest("POST", setup, {})) as { session: string };
    await demoRequest("POST", `${setup}/commit`, {
      session,
      parts: [
        { name: "Fan 1", kind: "fan", leds: 24 },
        { name: "Fan 2", kind: "fan", leds: 24 },
      ],
    });
    const { devices } = (await demoRequest("GET", "/api/v1/devices", undefined)) as {
      devices: { id: string; zones: { index: number; led_count: number }[] }[];
    };
    const zone = devices.find((d) => d.id === chain.device_id)?.zones.find((z) => z.index === chain.zone_index);
    expect(zone?.led_count).toBe(48);
  });
});
