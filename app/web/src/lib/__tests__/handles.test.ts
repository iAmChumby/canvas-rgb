// The stage's handles read from the effect's own schema (#257 phase 3, spec §4a). This runs the
// mapping over every built-in schema, exactly as `crates/canvas-effects/src/*.rs` declares them
// (the demo's `EFFECTS` copies their keys and ranges): Gradient and Rainbow Wave have a direction
// and a speed, so they get one vector; Aurora, Spiral, Rain, Color Cycle and Breathing have only
// a speed, so they get a pill; Solid, White, Fire, Screen Ambience and Audio Spectrum get nothing.

import { describe, expect, it } from "vitest";
import { EFFECTS } from "@/lib/fixtures";
import {
  ANGLE_SNAP_DEG,
  ARROW_MAX_PX,
  ARROW_MIN_PX,
  angleFromPoint,
  arrowLength,
  formatSpeed,
  handleOf,
  normaliseAngle,
  numberOf,
  quantise,
  scrubSpeed,
  snapAngle,
  speedFraction,
  speedValue,
  stepSpeed,
  type SpeedSpec,
} from "@/lib/handles";
import type { PropertySchema } from "@/proto/PropertySchema";

const schemas = Object.fromEntries(EFFECTS.map((effect) => [effect.schema.id, effect.schema] as const));
const schemaOf = (id: string): PropertySchema => {
  const schema = schemas[id];
  if (!schema) throw new Error(`no demo schema for ${id}`);
  return schema;
};

/** What the spec says each built-in gets. */
const VECTORS = ["gradient", "rainbow_wave"];
const PILLS = ["aurora", "spiral", "rain", "color_cycle", "breathing"];

describe("handleOf over every built-in schema", () => {
  it("draws one vector for Gradient and Rainbow Wave, the first angle as the direction", () => {
    expect(handleOf(schemaOf("gradient"))).toEqual({
      kind: "vector",
      direction: { key: "angle", label: "Angle" },
      speed: { key: "scroll_speed", label: "Scroll Speed", min: 0, max: 100, step: 1, inverse: false },
    });
    expect(handleOf(schemaOf("rainbow_wave"))).toEqual({
      kind: "vector",
      direction: { key: "direction", label: "Direction" },
      speed: { key: "speed", label: "Speed", min: 0, max: 100, step: 1, inverse: false },
    });
  });

  it("draws a pill for the effects with only a speed, and Breathing's period is inverse", () => {
    for (const id of PILLS) {
      const handle = handleOf(schemaOf(id));
      expect(handle.kind, id).toBe("pill");
      if (handle.kind !== "pill") continue;
      expect(["speed", "scroll_speed", "period"]).toContain(handle.speed.key);
    }
    expect(handleOf(schemaOf("breathing"))).toMatchObject({
      kind: "pill",
      speed: { key: "period", min: 0.5, max: 20, step: 0.1, inverse: true },
    });
  });

  it("draws nothing for the effects with neither", () => {
    for (const id of ["solid", "white", "fire", "screen_ambience", "audio_spectrum"]) {
      expect(handleOf(schemaOf(id)), id).toEqual({ kind: "none" });
    }
  });

  it("covers every schema in the library with exactly one of the three answers", () => {
    expect(EFFECTS.map((effect) => effect.schema.id).sort()).toEqual([
      "audio_spectrum",
      "aurora",
      "breathing",
      "color_cycle",
      "fire",
      "gradient",
      "rain",
      "rainbow_wave",
      "screen_ambience",
      "solid",
      "spiral",
      "white",
    ]);
    for (const effect of EFFECTS) {
      const handle = handleOf(effect.schema);
      const id = effect.schema.id;
      const wanted = VECTORS.includes(id) ? "vector" : PILLS.includes(id) ? "pill" : "none";
      expect(handle.kind, id).toBe(wanted);
    }
  });

  it("has no handle without a schema", () => {
    expect(handleOf(null)).toEqual({ kind: "none" });
    expect(handleOf(undefined)).toEqual({ kind: "none" });
  });

  it("gives an HTML effect that declares only an angle a direction handle", () => {
    // An HTML effect writes its own schema; one with an angle and no speed still points somewhere.
    const html: PropertySchema = {
      id: "html:swirl",
      name: "Swirl",
      description: "A user effect.",
      needs: { screen: false, audio: false },
      properties: [
        { key: "ink", label: "Ink", kind: { type: "color", default: "#7fd4ff" } },
        { key: "angle", label: "Angle", kind: { type: "angle", default: 0 } },
      ],
      presets: [],
    };
    expect(handleOf(html)).toEqual({ kind: "direction", direction: { key: "angle", label: "Angle" } });
  });
});

describe("the vector's arrow and the speed's range", () => {
  const speed: SpeedSpec = { key: "speed", label: "Speed", min: 0, max: 100, step: 1, inverse: false };
  const period: SpeedSpec = { key: "period", label: "Period", min: 0.5, max: 20, step: 0.1, inverse: true };

  it("runs from 16 px at the minimum to 72 px at the maximum", () => {
    expect(arrowLength(0)).toBe(ARROW_MIN_PX);
    expect(arrowLength(1)).toBe(ARROW_MAX_PX);
    expect(arrowLength(0.5)).toBe((ARROW_MIN_PX + ARROW_MAX_PX) / 2);
    expect(arrowLength(-1)).toBe(ARROW_MIN_PX);
    expect(arrowLength(2)).toBe(ARROW_MAX_PX);
  });

  it("reads a period backwards, so the shortest period is the fastest", () => {
    expect(speedFraction(period, 20)).toBe(0);
    expect(speedFraction(period, 0.5)).toBe(1);
    expect(speedValue(period, 1)).toBe(0.5);
    expect(speedValue(period, 0)).toBe(20);
    expect(speedFraction(speed, 25)).toBe(0.25);
    expect(speedValue(speed, 0.25)).toBe(25);
  });

  it("rounds to the schema's step and holds the range", () => {
    expect(quantise(3.94, 0.5, 20, 0.1)).toBe(3.9);
    expect(quantise(-5, 0, 100, 1)).toBe(0);
    expect(quantise(500, 0, 100, 1)).toBe(100);
  });
});

describe("the direction", () => {
  it("reads 0 right and 90 down, as the canvas and the AngleDial do", () => {
    expect(angleFromPoint({ x: 100, y: 100 }, { x: 200, y: 100 })).toBe(0);
    expect(angleFromPoint({ x: 100, y: 100 }, { x: 100, y: 200 })).toBe(90);
    expect(angleFromPoint({ x: 100, y: 100 }, { x: 0, y: 100 })).toBe(180);
    expect(angleFromPoint({ x: 100, y: 100 }, { x: 100, y: 0 })).toBe(270);
    expect(normaliseAngle(360)).toBe(0);
    expect(normaliseAngle(-90)).toBe(270);
  });

  it("snaps a Shift drag to 15° steps", () => {
    expect(snapAngle(22)).toBe(15);
    expect(snapAngle(23)).toBe(30);
    expect(snapAngle(359)).toBe(0);
    // 112 is not a multiple of 15: the nearest step is 105.
    expect(snapAngle(112)).toBe(105);
    expect(snapAngle(120)).toBe(120);
    expect(ANGLE_SNAP_DEG).toBe(15);
  });
});

describe("the pill's scrub and keys", () => {
  const speed: SpeedSpec = { key: "speed", label: "Speed", min: 10, max: 400, step: 1, inverse: false };
  const period: SpeedSpec = { key: "period", label: "Period", min: 0.5, max: 20, step: 0.1, inverse: true };

  it("scrubs half the range over half a sweep, held inside it", () => {
    expect(scrubSpeed(speed, 10, 80)).toBe(205);
    expect(scrubSpeed(speed, 10, -500)).toBe(10);
    expect(scrubSpeed(speed, 10, 5000)).toBe(400);
  });

  it("steps right for faster, and a period steps down to go faster", () => {
    expect(stepSpeed(speed, 120, 1)).toBe(121);
    expect(stepSpeed(speed, 120, -1)).toBe(119);
    expect(stepSpeed(period, 4, 1)).toBe(3.9);
    expect(stepSpeed(period, 4, -1)).toBe(4.1);
    expect(stepSpeed(period, 0.5, 1)).toBe(0.5);
  });

  it("shows whole numbers whole and a tenth as one decimal", () => {
    expect(formatSpeed(120)).toBe("120");
    expect(formatSpeed(3.9)).toBe("3.9");
  });
});

describe("reading a prop's number", () => {
  it("reads an angle, a hue and a number alike", () => {
    expect(numberOf({ type: "angle", value: 45 })).toBe(45);
    expect(numberOf({ type: "hue", value: 120 })).toBe(120);
    expect(numberOf({ type: "number", value: 30 })).toBe(30);
    expect(numberOf(undefined, 7)).toBe(7);
    expect(numberOf({ type: "choice", value: "Mono" }, 7)).toBe(7);
  });
});
