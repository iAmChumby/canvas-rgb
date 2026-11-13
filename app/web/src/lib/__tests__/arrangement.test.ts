// The editor's arrangement planner: the TypeScript port of the daemon's
// crates/canvas-core/src/arrangement.rs. Every test here is the port of one Rust test, with
// the same numbers, and the last one checks a whole plan against the Rust planner's own
// output (arrangement.vectors.json, printed by the Rust example).
//
// The builder stage draws a group's arrangement with these rules, so the drawing the owner
// sees and the picture the daemon samples are the same picture.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CANVAS_HEIGHT,
  CANVAS_WIDTH,
  MAX_ZONE_LEDS,
  RING_INSET,
  buildGroupPlan,
  ledCount,
  lightLedCount,
  planLight,
  region,
} from "@/lib/arrangement";
import type { Arrangement } from "@/proto/Arrangement";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { GroupMode } from "@/proto/GroupMode";
import type { Light } from "@/proto/Light";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { ZoneKind } from "@/proto/ZoneKind";

function zone(index: number, kind: ZoneKind, ledCount: number): ZoneInfo {
  return {
    index,
    name: `zone${index}`,
    kind,
    led_count: ledCount,
    min_leds: ledCount,
    max_leds: ledCount,
    resizable: false,
  };
}

function device(id: string, zones: ZoneInfo[]): DeviceInfo {
  return {
    id,
    name: id,
    vendor: "test",
    kind: "other",
    adapter: "fake",
    adapter_index: 0,
    serial: "",
    location: "",
    zones,
    connected: true,
  };
}

function light(id: string, deviceId: string, zoneIndex: number, arrangement: Arrangement): Light {
  return {
    id,
    device_id: deviceId,
    zone_index: zoneIndex,
    arrangement,
    reversed: false,
  };
}

/** A strip light on the first zone of its own device. */
function strip(id: string, deviceId: string): Light {
  return light(id, deviceId, 0, "strip");
}

function ids(list: string[]): string[] {
  return list;
}

/** The first item of a list, or a thrown error: the tests know their lists are not empty. */
function first<T>(list: readonly T[]): T {
  const item = list[0];
  if (item === undefined) throw new Error("the list is empty");
  return item;
}

/**
 * Fails the test with the message, like an assert.
 *
 * The property tests check every LED of hundreds of plans, and one `expect` per LED is
 * slower than the test's own time limit; the check itself is the same assertion.
 */
function check(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

/** Checks that a sample's bounds hold its centre and are as wide and as tall as the cell. */
function expectCell(
  led: { center_x: number; center_y: number; bounds: [number, number, number, number] },
  cellW: number,
  cellH: number,
  what: string,
): void {
  const [x0, y0, x1, y1] = led.bounds;
  expect(x0 <= led.center_x && led.center_x <= x1, `${what}: the centre x is not in the bounds`).toBe(
    true,
  );
  expect(y0 <= led.center_y && led.center_y <= y1, `${what}: the centre y is not in the bounds`).toBe(
    true,
  );
  expect(Math.abs(x1 - x0 - cellW), `${what}: the cell is ${x1 - x0} wide, not ${cellW}`).toBeLessThan(
    0.05,
  );
  expect(Math.abs(y1 - y0 - cellH), `${what}: the cell is ${y1 - y0} tall, not ${cellH}`).toBeLessThan(
    0.05,
  );
}

describe("the arrangement planner (#310)", () => {
  it("gives the slots by led share in flow across", () => {
    const devices = [
      device("d0", [zone(0, { type: "linear" }, 12)]),
      device("d1", [zone(0, { type: "linear" }, 36)]),
    ];
    const lights = [strip("a", "d0"), strip("b", "d1")];
    const plan = buildGroupPlan(ids(["a", "b"]), "flow_across", lights, devices);
    expect(plan.lights.length).toBe(2);
    const [first, second] = plan.lights;
    expect(first?.light).toBe("a");
    expect(second?.light).toBe("b");
    // 12 of the group's 48 LEDs: the first slot is a third of the canvas.
    for (const led of first?.leds ?? []) {
      expect(led.center_x < 80.0, `the first light's LED sits at ${led.center_x}`).toBe(true);
    }
    for (const led of second?.leds ?? []) {
      expect(led.center_x >= 80.0, `the second light's LED sits at ${led.center_x}`).toBe(true);
    }
  });

  it("gives every light the whole canvas in each its own", () => {
    const devices = [
      device("d0", [zone(0, { type: "linear" }, 12)]),
      device("d1", [zone(0, { type: "linear" }, 36)]),
    ];
    const lights = [strip("a", "d0"), strip("b", "d1")];
    const plan = buildGroupPlan(ids(["a", "b"]), "each_its_own", lights, devices);
    expect(plan.lights.length).toBe(2);
    for (const light of plan.lights) {
      // The slot is the whole canvas: the first cell starts at 0 and the last ends at 320,
      // so the light covers the whole picture and not a part of it.
      const firstCell = first(light.leds);
      const lastCell = light.leds[light.leds.length - 1];
      expect(Math.abs(firstCell.bounds[0]), `${light.light} starts at ${firstCell.bounds[0]}`).toBeLessThan(
        0.05,
      );
      expect(
        Math.abs(lastCell!.bounds[2] - CANVAS_WIDTH),
        `${light.light} ends at ${lastCell!.bounds[2]}`,
      ).toBeLessThan(0.05);
      expect(Math.abs(firstCell.bounds[1]), `${light.light} starts at ${firstCell.bounds[1]}`).toBeLessThan(
        0.05,
      );
      expect(
        Math.abs(lastCell!.bounds[3] - CANVAS_HEIGHT),
        `${light.light} ends at ${lastCell!.bounds[3]}`,
      ).toBeLessThan(0.05);
    }
  });

  it("runs a strip left to right at mid-height", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 8)])];
    const lights = [strip("a", "d0")];
    const plan = buildGroupPlan(ids(["a"]), "flow_across", lights, devices);
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(8);
    let last = Number.NEGATIVE_INFINITY;
    for (const led of leds) {
      expect(led.center_x > last, `the xs ascend: ${last} then ${led.center_x}`).toBe(true);
      expect(Math.abs(led.center_y - 100.0), `the y is mid-height: ${led.center_y}`).toBeLessThan(0.01);
      last = led.center_x;
    }
  });

  it("starts a ring at the top and goes clockwise", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 8)])];
    const lights = [light("a", "d0", 0, "ring")];
    const plan = buildGroupPlan(ids(["a"]), "each_its_own", lights, devices);
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(8);
    const cx = 160.0;
    const cy = 100.0;
    const led0 = first(leds);
    expect(Math.abs(led0.center_x - cx), `LED 0 sits at ${led0.center_x}`).toBeLessThan(0.01);
    expect(led0.center_y < cy, `LED 0 sits at ${led0.center_y}`).toBe(true);
    // A quarter of the way round the circle is right of the centre.
    const led2 = leds[2];
    expect(led2!.center_x > cx, `LED 2 sits at ${led2!.center_x}`).toBe(true);
    expect(Math.abs(led2!.center_y - cy), `LED 2 sits at ${led2!.center_y}`).toBeLessThan(0.01);
  });

  it("keeps a ring's inset from the slot", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 8)])];
    const lights = [light("a", "d0", 0, "ring")];
    const plan = buildGroupPlan(ids(["a"]), "each_its_own", lights, devices);
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(8);
    // LED 0 is at the top of the circle, 4 units from the top of the slot.
    const led0 = first(leds);
    expect(Math.abs(led0.center_x - 160.0), `LED 0 sits at ${led0.center_x}`).toBeLessThan(0.01);
    expect(Math.abs(led0.center_y - 4.0), `LED 0 sits at ${led0.center_y}`).toBeLessThan(0.01);
    // And the bottom of the circle keeps the same inset.
    const led4 = leds[4];
    expect(Math.abs(led4!.center_y - 196.0), `LED 4 sits at ${led4!.center_y}`).toBeLessThan(0.01);
  });

  it("collapses a ring in a narrow slot to its centre", () => {
    const devices = [
      device("d0", [zone(0, { type: "linear" }, 1)]),
      device("d1", [zone(0, { type: "linear" }, 99)]),
    ];
    const lights = [light("a", "d0", 0, "ring"), strip("b", "d1")];
    const plan = buildGroupPlan(ids(["a", "b"]), "flow_across", lights, devices);
    // The ring's slot is a 3.2 unit slice of the canvas: too narrow for a circle.
    const led = first(first(plan.lights).leds);
    expect(Math.abs(led.center_x - 1.6), `the LED sits at ${led.center_x}`).toBeLessThan(0.01);
    expect(Math.abs(led.center_y - 100.0), `the LED sits at ${led.center_y}`).toBeLessThan(0.01);
  });

  it("flips the led order when reversed", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 6)])];
    const reversed = { ...strip("a", "d0"), reversed: true };
    const plan = buildGroupPlan(ids(["a"]), "flow_across", [reversed], devices);
    const leds = first(plan.lights).leds;
    let last = Number.POSITIVE_INFINITY;
    for (const led of leds) {
      expect(led.center_x < last, `the xs descend: ${last} then ${led.center_x}`).toBe(true);
      last = led.center_x;
    }
  });

  it("reads the slot's centre for a single", () => {
    const devices = [
      device("d0", [zone(0, { type: "linear" }, 12)]),
      device("d1", [zone(0, { type: "linear" }, 36)]),
    ];
    const lights = [light("a", "d0", 0, "single"), strip("b", "d1")];
    const plan = buildGroupPlan(ids(["a", "b"]), "flow_across", lights, devices);
    const firstLight = first(plan.lights);
    expect(firstLight.leds.length).toBe(12);
    for (const led of firstLight.leds) {
      // The first slot is 0..80, so its centre is at x=40.
      expect(Math.abs(led.center_x - 40.0), `the LED sits at ${led.center_x}`).toBeLessThan(0.01);
      expect(Math.abs(led.center_y - 100.0), `the LED sits at ${led.center_y}`).toBeLessThan(0.01);
    }
  });

  it("puts a single light's leds on one point", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 12)])];
    const lights = [light("a", "d0", 0, "single")];
    const plan = buildGroupPlan(ids(["a"]), "each_its_own", lights, devices);
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(12);
    for (const led of leds) {
      expect(Math.abs(led.center_x - 160.0), `the LED sits at ${led.center_x}`).toBeLessThan(0.01);
      expect(Math.abs(led.center_y - 100.0), `the LED sits at ${led.center_y}`).toBeLessThan(0.01);
    }
  });

  it("holds every sample's centre in its own cell", () => {
    const width = CANVAS_WIDTH;
    const height = CANVAS_HEIGHT;

    // A strip alone: the slot is the whole canvas, so each of its 8 cells is a
    // 320/8 = 40 wide column of the whole height.
    {
      const devices = [device("d0", [zone(0, { type: "linear" }, 8)])];
      const plan = buildGroupPlan(ids(["a"]), "each_its_own", [strip("a", "d0")], devices);
      for (const led of first(plan.lights).leds) {
        expectCell(led, width / 8.0, height, "a strip cell");
      }
    }

    // A grid alone on a 4x4 matrix: each cell is 320/4 by 200/4.
    {
      const map = Array.from({ length: 16 }, (_, i) => i);
      const devices = [device("d0", [zone(0, { type: "matrix", width: 4, height: 4, map }, 16)])];
      const plan = buildGroupPlan(ids(["a"]), "each_its_own", [light("a", "d0", 0, "grid")], devices);
      for (const led of first(plan.lights).leds) {
        expectCell(led, width / 4.0, height / 4.0, "a grid cell");
      }
    }

    // A single light alone: it reads the slot's centre, and its cell is one logical
    // unit there, not the whole slot. A single point has no spread to average over,
    // so the smallest cell the planner allows is the honest one.
    {
      const devices = [device("d0", [zone(0, { type: "linear" }, 1)])];
      const plan = buildGroupPlan(ids(["a"]), "each_its_own", [light("a", "d0", 0, "single")], devices);
      const led = first(first(plan.lights).leds);
      expectCell(led, 1.0, 1.0, "a single light's cell");
      expect(Math.abs(led.center_x - width / 2.0), `the centre x is ${led.center_x}`).toBeLessThan(
        0.01,
      );
      expect(Math.abs(led.center_y - height / 2.0), `the centre y is ${led.center_y}`).toBeLessThan(
        0.01,
      );
      const [x0, y0, x1, y1] = led.bounds;
      expect(Math.abs(x0 - (width / 2.0 - 0.5)), `the cell starts at x = ${x0}`).toBeLessThan(0.01);
      expect(Math.abs(x1 - (width / 2.0 + 0.5)), `the cell ends at x = ${x1}`).toBeLessThan(0.01);
      expect(Math.abs(y0 - (height / 2.0 - 0.5)), `the cell starts at y = ${y0}`).toBeLessThan(0.01);
      expect(Math.abs(y1 - (height / 2.0 + 0.5)), `the cell ends at y = ${y1}`).toBeLessThan(0.01);
    }

    // A ring alone: the cell of each LED is the square of its share of the circle.
    {
      const devices = [device("d0", [zone(0, { type: "linear" }, 8)])];
      const plan = buildGroupPlan(ids(["a"]), "each_its_own", [light("a", "d0", 0, "ring")], devices);
      const radius = (Math.min(height, width) - 2 * RING_INSET) / 2;
      const side = (Math.PI * 2 * radius) / 8;
      for (const led of first(plan.lights).leds) {
        const [x0, y0, x1, y1] = led.bounds;
        expect(
          x0 <= led.center_x && led.center_x <= x1,
          `a ring cell: the centre x ${led.center_x} is not in ${x0}..${x1}`,
        ).toBe(true);
        expect(
          y0 <= led.center_y && led.center_y <= y1,
          `a ring cell: the centre y ${led.center_y} is not in ${y0}..${y1}`,
        ).toBe(true);
        expect(
          Math.abs(x1 - x0 - side),
          `a ring cell: the cell is ${x1 - x0} wide, not ${side}`,
        ).toBeLessThan(0.05);
        // The circle's top and bottom LEDs have their cell cut off by the canvas.
        expect(
          y1 - y0 <= side + 0.05 && y1 - y0 >= 1.0,
          `a ring cell: the cell is ${y1 - y0} tall`,
        ).toBe(true);
      }
    }
  });

  it("fills the slot with a grid on a matrix and skips the gaps", () => {
    const map = [0, 1, null, 3];
    const devices = [device("d0", [zone(0, { type: "matrix", width: 2, height: 2, map }, 4)])];
    const lights = [light("a", "d0", 0, "grid")];
    const plan = buildGroupPlan(ids(["a"]), "each_its_own", lights, devices);
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(4);
    // The gap LED has no cell: it is black and never sampled.
    expect(region(leds[2]!)).toBeUndefined();
    // LED 0 is the top-left cell of the canvas, LED 3 the bottom-right.
    const led0 = first(leds);
    expect(Math.abs(led0.center_x - 80.0), `LED 0 sits at ${led0.center_x}`).toBeLessThan(0.01);
    expect(Math.abs(led0.center_y - 50.0), `LED 0 sits at ${led0.center_y}`).toBeLessThan(0.01);
    const led1 = leds[1];
    expect(Math.abs(led1!.center_x - 240.0), `LED 1 sits at ${led1!.center_x}`).toBeLessThan(0.01);
    const led3 = leds[3];
    expect(Math.abs(led3!.center_x - 240.0), `LED 3 sits at ${led3!.center_x}`).toBeLessThan(0.01);
    expect(Math.abs(led3!.center_y - 150.0), `LED 3 sits at ${led3!.center_y}`).toBeLessThan(0.01);
  });

  it("falls back to a strip when a grid is on a linear zone", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 4)])];
    const lights = [light("a", "d0", 0, "grid")];
    const plan = buildGroupPlan(ids(["a"]), "each_its_own", lights, devices);
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(4);
    // A strip: left to right at mid-height.
    const led0 = first(leds);
    expect(Math.abs(led0.center_x - 40.0), `LED 0 sits at ${led0.center_x}`).toBeLessThan(0.01);
    expect(Math.abs(led0.center_y - 100.0), `LED 0 sits at ${led0.center_y}`).toBeLessThan(0.01);
    const led3 = leds[3];
    expect(Math.abs(led3!.center_x - 280.0), `LED 3 sits at ${led3!.center_x}`).toBeLessThan(0.01);
  });

  it("uses only a slice's run and its frame start", () => {
    const devices = [
      device("d0", [zone(0, { type: "linear" }, 8), zone(1, { type: "linear" }, 40)]),
    ];
    const sliced = { ...strip("a", "d0"), zone_index: 1, leds: { start: 12, count: 12 } };
    const plan = buildGroupPlan(ids(["a"]), "flow_across", [sliced], devices);
    const planLight = first(plan.lights);
    // The zone before it has 8 LEDs, so the slice starts at 8 + 12.
    expect(planLight.frame_start).toBe(20);
    expect(planLight.leds.length).toBe(12);
  });

  it("skips a member whose device is gone, without throwing", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 12)])];
    const lights = [strip("a", "d0"), strip("ghost", "gone")];
    const plan = buildGroupPlan(ids(["a", "ghost", "nothing"]), "flow_across", lights, devices);
    expect(plan.lights.length).toBe(1);
    expect(first(plan.lights).light).toBe("a");
    // The gone light has no LEDs at all.
    expect(lightLedCount(lights[1]!, devices)).toBe(0);
  });

  it("gives a zone with no leds no slot", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 0)])];
    const lights = [strip("a", "d0")];
    expect(lightLedCount(lights[0]!, devices)).toBe(0);
    const plan = buildGroupPlan(ids(["a"]), "flow_across", lights, devices);
    expect(plan.lights.length).toBe(0);
    expect(ledCount(plan)).toBe(0);
  });

  it("gives a member listed twice one slot", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 12)])];
    const lights = [strip("a", "d0")];
    const plan = buildGroupPlan(ids(["a", "a"]), "flow_across", lights, devices);
    expect(plan.lights.length).toBe(1);
    expect(first(plan.lights).light).toBe("a");
    expect(first(plan.lights).leds.length).toBe(12);
    expect(ledCount(plan)).toBe(12);
  });

  it("caps a zone that reports too many leds", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 100000)])];
    const lights = [strip("a", "d0")];
    expect(lightLedCount(lights[0]!, devices)).toBe(MAX_ZONE_LEDS);
    const plan = buildGroupPlan(ids(["a"]), "flow_across", lights, devices);
    expect(first(plan.lights).leds.length).toBe(MAX_ZONE_LEDS);
    expect(ledCount(plan)).toBe(MAX_ZONE_LEDS);
  });

  it("does not overflow the sum of two huge zones", () => {
    const devices = [
      device("d0", [zone(0, { type: "linear" }, 3000000000)]),
      device("d1", [zone(0, { type: "linear" }, 3000000000)]),
    ];
    const lights = [strip("a", "d0"), strip("b", "d1")];
    const plan = buildGroupPlan(ids(["a", "b"]), "flow_across", lights, devices);
    expect(ledCount(plan)).toBe(2 * MAX_ZONE_LEDS);
    // The two slots share the canvas: the first holds the left half.
    const firstLed = first(first(plan.lights).leds);
    const secondLed = first(plan.lights[1]!.leds);
    expect(firstLed.center_x < 160.0, `the first light sits at ${firstLed.center_x}`).toBe(true);
    expect(secondLed.center_x > 160.0, `the second light sits at ${secondLed.center_x}`).toBe(true);
  });

  it("starts a second zone after the first zone's real leds", () => {
    const firstZone = 100000;
    const devices = [
      device("d0", [
        zone(0, { type: "linear" }, firstZone),
        zone(1, { type: "linear" }, 12),
      ]),
    ];
    const second = { ...strip("b", "d0"), zone_index: 1 };
    const lights = [strip("a", "d0"), second];
    const plan = buildGroupPlan(ids(["a", "b"]), "each_its_own", lights, devices);

    // The frame the engine builds for this device holds every real LED of both zones.
    const frame = devices[0]!.zones.reduce((sum, z) => sum + z.led_count, 0);
    expect(frame).toBe(firstZone + 12);
    // The second zone starts after the first zone's real 100,000 LEDs, not after the
    // 4,096 the planner samples.
    const secondLight = planLight(plan, "b");
    expect(secondLight?.frame_start).toBe(firstZone);
    expect(secondLight?.leds.length).toBe(12);

    // The first zone is planned as the cap's worth of samples...
    const firstLight = planLight(plan, "a");
    expect(firstLight?.frame_start).toBe(0);
    expect(firstLight?.leds.length).toBe(MAX_ZONE_LEDS);
    // ...and the LEDs past the cap have no sample: nothing in the plan covers frame
    // index 4096, nor any index between the cap and the second zone's start.
    for (const ledIndex of [MAX_ZONE_LEDS, firstZone - 1]) {
      const covered = plan.lights.some((light) => {
        const end = light.frame_start + light.leds.length;
        return light.frame_start <= ledIndex && ledIndex < end;
      });
      expect(covered, `LED ${ledIndex} of the device frame has a sample`).toBe(false);
    }
  });

  it("clamps a slice that runs past its zone", () => {
    const devices = [device("d0", [zone(0, { type: "linear" }, 20)])];
    const sliced = { ...strip("a", "d0"), leds: { start: 16, count: 12 } };
    // The zone only has 4 LEDs left of the slice.
    expect(lightLedCount(sliced, devices)).toBe(4);
    const plan = buildGroupPlan(ids(["a"]), "flow_across", [sliced], devices);
    expect(first(plan.lights).leds.length).toBe(4);
    // The slice still starts at 16 in the device's frame.
    expect(first(plan.lights).frame_start).toBe(16);
  });

  it("maps only the leds of a grid light's slice", () => {
    const map = Array.from({ length: 16 }, (_, i) => i);
    const devices = [
      device("d0", [zone(0, { type: "matrix", width: 4, height: 4, map }, 16)]),
    ];
    const sliced = { ...light("a", "d0", 0, "grid"), leds: { start: 12, count: 4 } };
    const plan = buildGroupPlan(ids(["a"]), "each_its_own", [sliced], devices);
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(4);
    // The slice is the bottom row of the matrix: y is below the centre.
    for (const led of leds) {
      expect(led.center_y > 100.0, `the LED sits at ${led.center_y}`).toBe(true);
    }
    // And the first LED of the slice is the bottom-left cell.
    const led0 = first(leds);
    expect(Math.abs(led0.center_x - 40.0), `LED 0 sits at ${led0.center_x}`).toBeLessThan(0.01);
    expect(Math.abs(led0.center_y - 175.0), `LED 0 sits at ${led0.center_y}`).toBeLessThan(0.01);
  });

  it("plans a damaged matrix from its map, not its dimensions", () => {
    // 100000 x 100000 cells is 10^10 of them, and the map holds one LED: the planner walks
    // the map, so the plan is one sample and takes no noticeable time.
    const devices = [
      device("d0", [zone(0, { type: "matrix", width: 100000, height: 100000, map: [0] }, 1)]),
    ];
    const lights = [light("a", "d0", 0, "grid")];
    const started = performance.now();
    const plan = buildGroupPlan(ids(["a"]), "each_its_own", lights, devices);
    const elapsed = performance.now() - started;
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(1);
    expect(region(first(leds))).toBeDefined();
    expect(elapsed, `the plan took ${elapsed} ms`).toBeLessThan(50);
  });

  it("plans the leds a map shorter than the matrix names", () => {
    // A 3x3 matrix whose map holds only its top row.
    const map = [0, 1, 2];
    const devices = [device("d0", [zone(0, { type: "matrix", width: 3, height: 3, map }, 3)])];
    const lights = [light("a", "d0", 0, "grid")];
    const plan = buildGroupPlan(ids(["a"]), "each_its_own", lights, devices);
    const leds = first(plan.lights).leds;
    expect(leds.length).toBe(3);
    let last = Number.NEGATIVE_INFINITY;
    for (const led of leds) {
      expect(region(led), "a named LED has a cell").toBeDefined();
      // The three LEDs are the matrix's top row: the cells of 3x3.
      expect(Math.abs(led.center_y - 200.0 / 6.0), `the LED sits at ${led.center_y}`).toBeLessThan(
        0.01,
      );
      expect(led.center_x > last, `the xs ascend: ${last} then ${led.center_x}`).toBe(true);
      last = led.center_x;
    }
  });

  it("keeps every sample inside the canvas", () => {
    for (let n = 1; n <= 300; n++) {
      const devices = [device("d0", [zone(0, { type: "linear" }, n)])];
      for (const arrangement of ["single", "strip", "ring", "grid"] as const) {
        const sample = { ...light("a", "d0", 0, arrangement), reversed: n % 2 === 0 };
        for (const mode of ["flow_across", "each_its_own"] as const) {
          const plan = buildGroupPlan(ids(["a"]), mode, [sample], devices);
          for (const led of first(plan.lights).leds) {
            const [x0, y0, x1, y1] = led.bounds;
            const what = `n=${n} ${arrangement} ${mode}`;
            check(x0 >= 0.0 && x1 <= CANVAS_WIDTH, `${what}: the x is ${x0}..${x1}`);
            check(y0 >= 0.0 && y1 <= CANVAS_HEIGHT, `${what}: the y is ${y0}..${y1}`);
            check(x1 - x0 >= 1.0, `${what}: the cell is ${x1 - x0} wide`);
            check(y1 - y0 >= 1.0, `${what}: the cell is ${y1 - y0} tall`);
            check(
              led.center_x >= 0.0 && led.center_x <= CANVAS_WIDTH,
              `${what}: the centre x is ${led.center_x}`,
            );
            check(
              led.center_y >= 0.0 && led.center_y <= CANVAS_HEIGHT,
              `${what}: the centre y is ${led.center_y}`,
            );
          }
        }
      }
    }
  });

  it("keeps every matrix sample inside the canvas", () => {
    for (const [cols, rows] of [
      [1, 1],
      [2, 2],
      [3, 5],
      [10, 10],
      [17, 17],
      [25, 12],
    ] as const) {
      const n = cols * rows;
      if (n > 300) continue;
      // The zone's own map: cell (row, col) holds the LED of that number.
      const map = Array.from({ length: n }, (_, i) => i);
      const devices = [
        device("d0", [zone(0, { type: "matrix", width: cols, height: rows, map }, n)]),
      ];
      const sample = { ...light("a", "d0", 0, "grid"), reversed: n % 2 === 0 };
      for (const mode of ["flow_across", "each_its_own"] as const) {
        const plan = buildGroupPlan(ids(["a"]), mode, [sample], devices);
        for (const led of first(plan.lights).leds) {
          const [x0, y0, x1, y1] = led.bounds;
          const what = `${cols}x${rows} ${mode}`;
          check(x0 >= 0.0 && x1 <= CANVAS_WIDTH, `${what}: the x is ${x0}..${x1}`);
          check(y0 >= 0.0 && y1 <= CANVAS_HEIGHT, `${what}: the y is ${y0}..${y1}`);
          check(x1 - x0 >= 1.0, `${what}: the cell is ${x1 - x0} wide`);
          check(y1 - y0 >= 1.0, `${what}: the cell is ${y1 - y0} tall`);
          check(
            led.center_x >= 0.0 && led.center_x <= CANVAS_WIDTH,
            `${what}: the centre x is ${led.center_x}`,
          );
          check(
            led.center_y >= 0.0 && led.center_y <= CANVAS_HEIGHT,
            `${what}: the centre y is ${led.center_y}`,
          );
        }
      }
    }
  });
});

/** One case of the Rust planner's own output: the inputs and the plan they make. */
interface VectorCase {
  name: string;
  members: string[];
  mode: GroupMode;
  lights: Light[];
  devices: DeviceInfo[];
  plan: {
    lights: {
      light: string;
      device_id: string;
      frame_start: number;
      leds: {
        center_x: number;
        center_y: number;
        bounds: [number, number, number, number];
        sampled: boolean;
      }[];
    }[];
  };
}

/** The Rust planner's output for the fixed inputs, printed by the Rust example. */
interface Vectors {
  canvas: { width: number; height: number };
  cases: VectorCase[];
}

/**
 * Where the Rust planner's own output lives: next to this test, whether the tests run from
 * the editor's folder or from the repository root.
 */
function vectorsPath(): string {
  const names = [
    join(process.cwd(), "src/lib/__tests__/arrangement.vectors.json"),
    join(process.cwd(), "app/web/src/lib/__tests__/arrangement.vectors.json"),
  ];
  const found = names.find((name) => existsSync(name));
  if (!found) {
    throw new Error(`the Rust vectors are not there: looked in ${names.join(" and ")}`);
  }
  return found;
}

describe("the Rust planner's own output", () => {
  it("arrangement.ts matches the Rust vectors", () => {
    const vectors = JSON.parse(readFileSync(vectorsPath(), "utf8")) as Vectors;
    expect(vectors.canvas.width).toBe(CANVAS_WIDTH);
    expect(vectors.canvas.height).toBe(CANVAS_HEIGHT);
    expect(vectors.cases.length).toBeGreaterThan(0);
    // The largest gap between the two planners, over every case and every LED: the f32
    // Rust planner against the double TS one. One assertion at the end, so the number is
    // reported whether or not it passes.
    let maxDeviation = 0;
    let maxWhere = "";
    let ledsChecked = 0;
    const deviation = (got: number | undefined, want: number, where: string): void => {
      const d = Math.abs((got ?? 0) - want);
      if (d > maxDeviation) {
        maxDeviation = d;
        maxWhere = where;
      }
    };
    for (const testCase of vectors.cases) {
      const plan = buildGroupPlan(testCase.members, testCase.mode, testCase.lights, testCase.devices);
      const what = testCase.name;
      expect(plan.lights.length, `${what}: the plan holds ${plan.lights.length} lights`).toBe(
        testCase.plan.lights.length,
      );
      for (let i = 0; i < testCase.plan.lights.length; i++) {
        const got = plan.lights[i];
        const want = testCase.plan.lights[i]!;
        expect(got?.light, `${what}: light ${i}`).toBe(want.light);
        expect(got?.device_id, `${what}: light ${i}`).toBe(want.device_id);
        expect(got?.frame_start, `${what}: light ${i}`).toBe(want.frame_start);
        expect(got?.leds.length, `${what}: light ${i} holds ${got?.leds.length} LEDs`).toBe(
          want.leds.length,
        );
        for (let j = 0; j < want.leds.length; j++) {
          const gotLed = got?.leds[j];
          const wantLed = want.leds[j]!;
          const where = `${what}: light ${i} LED ${j}`;
          deviation(gotLed?.center_x, wantLed.center_x, `${where} centre x`);
          deviation(gotLed?.center_y, wantLed.center_y, `${where} centre y`);
          expect(gotLed?.sampled, `${where} sampled`).toBe(wantLed.sampled);
          for (let k = 0; k < 4; k++) {
            deviation(gotLed?.bounds[k], wantLed.bounds[k]!, `${where} bound ${k}`);
          }
          ledsChecked++;
        }
      }
    }
    console.log(
      `the largest deviation from the Rust planner over ${vectors.cases.length} cases and ${ledsChecked} LEDs is ${maxDeviation} (${maxWhere})`,
    );
    expect(
      maxDeviation,
      `the largest deviation from the Rust planner is ${maxDeviation} at ${maxWhere}`,
    ).toBeLessThanOrEqual(1e-4);
  });
});
