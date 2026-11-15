// The area picker's geometry (D95.6 test 17): pointer → canvas through the stage's zoom and pan,
// a drag that starts off the desk, and the readout's numbers.

import { describe, expect, it } from "vitest";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import {
  areaBounds,
  areaName,
  areaReadout,
  pointerToCanvas,
  rectFromDrag,
} from "@/views/effects/stackDraft";
import { canvasWith } from "@/lib/__tests__/groupCanvasFixture";

const desk = canvasWith([
  {
    id: "royuan:3151:4015#z0",
    label: "ROYUAN Keyboard · Keyboard",
    device: "royuan:3151:4015",
    zone: 0,
    x: 16,
    y: 158,
    width: 110,
    height: 30,
  },
]);

const devices: DeviceInfo[] = [
  {
    id: "royuan:3151:4015",
    name: "ROYUAN Keyboard",
    vendor: "ROYUAN",
    kind: "keyboard",
    adapter: "royuan",
    adapter_index: 0,
    serial: "",
    location: "",
    connected: true,
    zones: [],
  },
];

describe("pointer to canvas", () => {
  const box = { left: 10, top: 20, width: 640, height: 400 };

  it("inverts the stage's zoom and pan", () => {
    // base = 640 / 320 = 2; k = base x zoom = 4. With pan -40, canvas x = css/4 + 40.
    const at = pointerToCanvas(10 + 4 * (100 - 40), 20 + 4 * (50 - 10), box, { zoom: 2, panX: -40, panY: -10 });
    expect(at.x).toBeCloseTo(100, 6);
    expect(at.y).toBeCloseTo(50, 6);
  });

  it("clamps a pointer beside the desk to its edge", () => {
    expect(pointerToCanvas(-500, -500, box, { zoom: 1, panX: 0, panY: 0 })).toEqual({ x: 0, y: 0 });
    expect(pointerToCanvas(5000, 5000, box, { zoom: 1, panX: 0, panY: 0 })).toEqual({ x: 320, y: 200 });
  });
});

describe("the rectangle a drag draws", () => {
  it("normalises either direction", () => {
    expect(rectFromDrag({ x: 120, y: 60 }, { x: 20, y: 40 })).toEqual({ x: 20, y: 40, width: 100, height: 20 });
  });

  it("draws from the edge when the drag started off the desk", () => {
    const box = { left: 0, top: 0, width: 160, height: 100 };
    const outside = pointerToCanvas(-30, -30, box, { zoom: 1, panX: 0, panY: 0 });
    const inside = pointerToCanvas(20, 10, box, { zoom: 1, panX: 0, panY: 0 });
    expect(rectFromDrag(outside, inside)).toEqual({ x: 0, y: 0, width: 40, height: 20 });
  });

  it("keeps the rectangle on the canvas and at least one pixel", () => {
    expect(rectFromDrag({ x: 300, y: 190 }, { x: 400, y: 300 })).toEqual({ x: 300, y: 190, width: 20, height: 10 });
    expect(rectFromDrag({ x: 10, y: 10 }, { x: 10, y: 10 })).toEqual({ x: 10, y: 10, width: 1, height: 1 });
  });
});

describe("the readout", () => {
  it("says the size and where the rectangle sits", () => {
    const readout = areaReadout({ kind: "rect", x: 16, y: 150, width: 110, height: 46 }, desk, devices);
    expect(readout.title).toBe("Rectangle 110 × 46");
    expect(readout.detail).toBe("x 16, y 150");
  });

  it("names a device and a zone, and says when one is not in the group", () => {
    expect(areaReadout({ kind: "canvas" }, desk, devices).title).toBe("Whole canvas");
    expect(areaReadout({ kind: "device", device: "royuan:3151:4015" }, desk, devices).title).toBe("ROYUAN Keyboard");
    expect(areaReadout({ kind: "zone", device: "royuan:3151:4015", zone: 0 }, desk, devices).title).toBe(
      "ROYUAN Keyboard · Keyboard",
    );
    expect(areaName({ kind: "device", device: "gone" }, desk, devices)).toBe("Not in this group");
  });

  it("takes a device's area as the union of its lights' boxes", () => {
    const bounds = areaBounds({ kind: "device", device: "royuan:3151:4015" }, desk);
    expect(bounds).toEqual({ x: 16, y: 158, width: 110, height: 30 });
    expect(areaBounds({ kind: "device", device: "gone" }, desk)).toBeNull();
    expect(areaBounds({ kind: "rect", x: 16, y: 150, width: 110, height: 46 }, desk)).toEqual({
      x: 16,
      y: 150,
      width: 110,
      height: 46,
    });
  });
});
