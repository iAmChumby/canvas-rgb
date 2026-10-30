import { describe, expect, it } from "vitest";
import {
  dragThresholdPx,
  hitStack,
  hitTest,
  marqueePick,
  partsInsideCase,
  type HitInput,
  type HitPart,
} from "@/lib/hitTest";

// 4.5 css pixels to a canvas unit: the zoom the 1440 desk opens at, near enough.
const K = 4.5;

function part(
  kind: "zone" | "landmark",
  index: number,
  x: number,
  y: number,
  width: number,
  height: number,
  extra: Partial<HitPart> = {},
): HitPart {
  return { ref: { kind, index }, x, y, width, height, rotation_deg: 0, isCase: false, locked: false, ...extra };
}

const caseBox = (index: number, extra: Partial<HitPart> = {}): HitPart =>
  part("landmark", index, 100, 27, 120, 122, { isCase: true, ...extra });

function input(parts: HitPart[], x: number, y: number, extra: Partial<HitInput> = {}): HitInput {
  return { point: { x, y }, parts, selection: [], k: K, ...extra };
}

describe("hitTest: the part under the pointer wins over the case", () => {
  const fan = part("zone", 0, 120, 60, 30, 30);
  const cpu = part("landmark", 1, 170, 90, 16, 16);

  it("picks a part inside the case when the case was added first", () => {
    const parts = [fan, part("landmark", 0, 100, 27, 120, 122, { isCase: true }), cpu];
    expect(hitTest(input(parts, 135, 75)).kind).toBe("part");
    expect(hitTest(input(parts, 178, 98))).toMatchObject({ kind: "part", ref: { kind: "landmark", index: 1 } });
  });

  it("picks a part inside the case when the case was added last", () => {
    // The landmarks array is [cpu, case]: DOM order used to put the case on top of the cpu.
    const cpuFirst = part("landmark", 0, 170, 90, 16, 16);
    const parts = [fan, cpuFirst, caseBox(1)];
    expect(hitTest(input(parts, 178, 98))).toMatchObject({ kind: "part", ref: { kind: "landmark", index: 0 } });
    expect(hitTest(input(parts, 135, 75))).toMatchObject({ kind: "part", ref: { kind: "zone", index: 0 } });
  });

  it("gives the same answer whatever order the parts are listed in", () => {
    const a = [fan, cpu, caseBox(2)];
    const b = [caseBox(2), cpu, fan];
    for (const [x, y] of [
      [135, 75],
      [178, 98],
      [110, 140],
      [101, 80],
    ] as const) {
      expect(hitTest(input(a, x, y))).toEqual(hitTest(input(b, x, y)));
    }
  });

  it("hits an offline zone exactly like a connected one", () => {
    const offline = part("zone", 3, 190, 40, 6, 34, { offline: true });
    const parts = [offline, caseBox(0)];
    expect(hitTest(input(parts, 193, 60))).toMatchObject({ kind: "part", ref: { kind: "zone", index: 3 } });
  });

  it("hits a locked part and says it is locked", () => {
    const locked = part("zone", 0, 120, 60, 30, 30, { locked: true });
    const hit = hitTest(input([locked, caseBox(0)], 135, 75));
    expect(hit).toMatchObject({ kind: "part", locked: true });
  });

  it("prefers the smallest part where two overlap", () => {
    const gpu = part("landmark", 1, 110, 100, 70, 14);
    const pump = part("zone", 0, 120, 98, 16, 16);
    const parts = [pump, gpu, caseBox(0)];
    expect(hitTest(input(parts, 128, 106))).toMatchObject({ ref: { kind: "zone", index: 0 } });
    expect(hitTest(input(parts, 160, 106))).toMatchObject({ ref: { kind: "landmark", index: 1 } });
  });

  it("gives a thin part a usable target", () => {
    // A 2 unit strip is 9 px at this zoom: the hit reaches a few pixels past it.
    const thin = part("zone", 0, 110, 60, 80, 2);
    expect(hitTest(input([thin, caseBox(0)], 150, 62.2)).kind).toBe("part");
    expect(hitTest(input([thin, caseBox(0)], 150, 70)).kind).toBe("case-body");
  });
});

describe("hitTest: the case", () => {
  const c = caseBox(0);

  it("is taken by an empty spot inside it", () => {
    expect(hitTest(input([c], 160, 90))).toMatchObject({ kind: "case-body", ref: { kind: "landmark", index: 0 } });
  });

  it("is taken by its edge band", () => {
    // 8 px at 4.5 px a unit is 1.78 units.
    expect(hitTest(input([c], 100.8, 90)).kind).toBe("case-edge");
    expect(hitTest(input([c], 160, 148.5)).kind).toBe("case-edge");
    expect(hitTest(input([c], 160, 90 + 0)).kind).toBe("case-body");
  });

  it("leaves the empty desk outside it", () => {
    expect(hitTest(input([c], 50, 90))).toEqual({ kind: "desk" });
    expect(hitTest(input([], 50, 90))).toEqual({ kind: "desk" });
  });

  it("measures the edge band on a rotated case in its own frame", () => {
    // A case turned 90 degrees: its box on screen is 122 wide and 120 tall around the same centre.
    const turned = caseBox(0, { rotation_deg: 90 });
    const cx = 160;
    const cy = 88;
    // Its screen left edge is at cx - 61. Just inside it is the edge band; the same offset from
    // the unrotated left edge (cx - 60) is not an edge any more but is still the body.
    expect(hitTest(input([turned], cx - 61 + 0.8, cy)).kind).toBe("case-edge");
    // Along the screen-top edge of a 90 degree case: y = cy - 60.
    expect(hitTest(input([turned], cx, cy - 60 + 0.8)).kind).toBe("case-edge");
    // Inside the axis-aligned box of a turned case but past the long side: not the case at all.
    const tilted = caseBox(0, { rotation_deg: 45 });
    // The tilted case's top-left corner of its AABB is empty desk.
    const aabbCorner = { x: 160 - 84, y: 88 - 85 };
    expect(hitTest(input([tilted], aabbCorner.x + 2, aabbCorner.y + 2))).toEqual({ kind: "desk" });
    // Its centre is the body.
    expect(hitTest(input([tilted], 160, 88)).kind).toBe("case-body");
  });

  it("does not call the interior of an unrotated band an edge on a rotated case", () => {
    // 45 degrees: a point 3 units in from the AABB's left side is far from any real edge.
    const tilted = caseBox(0, { rotation_deg: 45 });
    expect(hitTest(input([tilted], 160 - 84 + 40, 88)).kind).toBe("case-body");
  });
});

describe("hitTest: rotated parts", () => {
  it("hits a turned fan where it is, not where its unturned box was", () => {
    // A long strip turned 90 degrees stands up on the desk.
    const strip = part("zone", 0, 110, 85, 100, 6, { rotation_deg: 90 });
    const parts = [strip, caseBox(1)];
    // Its centre is (160, 88). Turned, it runs from y = 38 to y = 138.
    expect(hitTest(input(parts, 160, 50)).kind).toBe("part");
    expect(hitTest(input(parts, 160, 130)).kind).toBe("part");
    // Where the unturned strip used to run, off its new line, is the case body.
    expect(hitTest(input(parts, 115, 88)).kind).toBe("case-body");
  });

  it("hits a part turned 45 degrees at its corner", () => {
    const fan = part("zone", 0, 140, 70, 40, 40, { rotation_deg: 45 });
    const parts = [fan, caseBox(1)];
    // The unturned corner (140.5, 70.5) is outside the turned square; its turned tip is at
    // (160, 90 - 28.28) above the centre (160, 90).
    expect(hitTest(input(parts, 160, 90 - 27)).kind).toBe("part");
    expect(hitTest(input(parts, 141.5, 71.5)).kind).toBe("case-body");
  });
});

describe("hitTest: handles of the selected part", () => {
  const c = caseBox(0);
  const sel = [{ kind: "landmark" as const, index: 0 }];

  it("wins over a fan sitting under the case's edge handle", () => {
    // The fan is 3 units inside the front edge, level with the middle of the case's left edge.
    const fan = part("zone", 0, 103, 73, 30, 30);
    const parts = [fan, c];
    const midLeft = { x: 100, y: 27 + 61 };
    const onHandle = hitTest(input(parts, midLeft.x + 0.5, midLeft.y, { selection: sel }));
    expect(onHandle).toMatchObject({ kind: "handle", handle: { id: "w" } });
    // Press on the fan, a handle's width away: the fan.
    const onFan = hitTest(input(parts, midLeft.x + 12, midLeft.y, { selection: sel }));
    expect(onFan).toMatchObject({ kind: "part", ref: { kind: "zone", index: 0 } });
  });

  it("finds the corner handles", () => {
    const hit = hitTest(input([c], 220, 149, { selection: sel }));
    expect(hit).toMatchObject({ kind: "handle", handle: { id: "se" } });
  });

  it("finds the rotation grip above the top edge", () => {
    const hit = hitTest(input([c], 160, 27 - 24 / K, { selection: sel }));
    expect(hit).toMatchObject({ kind: "handle", handle: "rotate" });
  });

  it("follows the part's turn", () => {
    const turned = caseBox(0, { rotation_deg: 90 });
    // Turned 90 degrees clockwise, the box's local "e" handle (right edge) points down.
    const cx = 160;
    const cy = 88;
    const hit = hitTest(input([turned], cx, cy + 60, { selection: sel }));
    expect(hit).toMatchObject({ kind: "handle", handle: { id: "e" } });
  });

  it("hides the edge handles on a part too small for them, as the frame does", () => {
    const tiny = part("zone", 0, 150, 80, 8, 8);
    const s = [{ kind: "zone" as const, index: 0 }];
    // 8 units is 36 px, under the 44 px an edge handle needs: only the corners stay.
    const hit = hitTest(input([tiny, c], 154, 80, { selection: s }));
    expect(hit.kind).not.toBe("handle");
    const corner = hitTest(input([tiny, c], 150, 80, { selection: s }));
    expect(corner).toMatchObject({ kind: "handle", handle: { id: "nw" } });
  });

  it("has no handles when more than one part is picked, or when the part is locked", () => {
    const two = [
      { kind: "landmark" as const, index: 0 },
      { kind: "zone" as const, index: 0 },
    ];
    const fan = part("zone", 0, 120, 60, 30, 30);
    expect(hitTest(input([fan, c], 220, 149, { selection: two })).kind).toBe("case-edge");
    const locked = caseBox(0, { locked: true });
    expect(hitTest(input([locked], 220, 149, { selection: sel })).kind).toBe("case-edge");
  });

  it("is reached by a finger from further away than by a mouse", () => {
    const off = 11 / K; // 11 px from the nw corner of the case
    const mouse = hitTest(input([c], 100 - off, 27, { selection: sel, pointer: "mouse" }));
    const touch = hitTest(input([c], 100 - off, 27, { selection: sel, pointer: "touch" }));
    expect(mouse.kind).toBe("desk");
    expect(touch).toMatchObject({ kind: "handle", handle: { id: "nw" } });
  });
});

describe("hitStack and Alt-click: the part underneath", () => {
  const fan = part("zone", 0, 130, 60, 30, 30);
  const gpu = part("landmark", 1, 110, 70, 70, 14);
  const c = caseBox(0);
  const parts = [fan, gpu, c];

  it("lists everything under the point, smallest first, the case last", () => {
    const stack = hitStack(input(parts, 140, 75));
    expect(stack.map((t) => `${t.ref.kind}:${t.ref.index}`)).toEqual([
      "zone:0",
      "landmark:1",
      "landmark:0",
    ]);
  });

  it("Alt-click takes the next one down from the picked part", () => {
    const at = { x: 140, y: 75 };
    // Nothing picked: the second in the stack.
    expect(hitTest(input(parts, at.x, at.y, { under: true }))).toMatchObject({ ref: { kind: "landmark", index: 1 } });
    // The fan is picked: the gpu.
    expect(
      hitTest(input(parts, at.x, at.y, { under: true, selection: [{ kind: "zone", index: 0 }] })),
    ).toMatchObject({ ref: { kind: "landmark", index: 1 } });
    // The gpu is picked: the case.
    expect(
      hitTest(input(parts, at.x, at.y, { under: true, selection: [{ kind: "landmark", index: 1 }] })),
    ).toMatchObject({ ref: { kind: "landmark", index: 0 }, kind: "case-body" });
    // The case is picked: round to the top again.
    expect(
      hitTest(input(parts, at.x, at.y, { under: true, selection: [{ kind: "landmark", index: 0 }] })),
    ).toMatchObject({ ref: { kind: "zone", index: 0 } });
  });

  it("Alt-click skips a handle of the picked part", () => {
    const hit = hitTest(
      input([c], 220, 149, { under: true, selection: [{ kind: "landmark", index: 0 }] }),
    );
    expect(hit.kind).toBe("case-edge");
  });

  it("Alt-click on empty desk is empty desk", () => {
    expect(hitTest(input(parts, 20, 20, { under: true }))).toEqual({ kind: "desk" });
  });
});

describe("partsInsideCase", () => {
  const fan = part("zone", 0, 110, 40, 30, 30);
  const cpu = part("landmark", 1, 170, 90, 16, 16);
  const outsideFan = part("zone", 1, 20, 40, 30, 30);
  const other = caseBox(2);

  it("carries zones and landmarks, never the case itself or another case", () => {
    const got = partsInsideCase(caseBox(0), [fan, cpu, outsideFan, caseBox(0), other]);
    expect(got).toEqual([
      { kind: "zone", index: 0 },
      { kind: "landmark", index: 1 },
    ]);
  });

  it("tests a turned case in its own frame", () => {
    // A 45 degree case's AABB reaches (76, 3) to (244, 173); its real corners are cut off.
    const tilted = caseBox(0, { rotation_deg: 45 });
    const inCorner = part("zone", 0, 80, 5, 6, 6); // inside the AABB, outside the tilted box
    const inMiddle = part("zone", 1, 150, 80, 6, 6);
    const got = partsInsideCase(tilted, [inCorner, inMiddle, tilted]);
    expect(got).toEqual([{ kind: "zone", index: 1 }]);
  });

  it("uses the middle of a turned part", () => {
    const turned = part("zone", 0, 90, 80, 40, 6, { rotation_deg: 90 }); // centre (110, 83): inside
    expect(partsInsideCase(caseBox(0), [turned])).toEqual([{ kind: "zone", index: 0 }]);
  });
});

describe("marqueePick", () => {
  const fan = part("zone", 0, 110, 40, 30, 30);
  const cpu = part("landmark", 1, 170, 90, 16, 16);
  const c = caseBox(2);

  it("picks landmarks as well as zones", () => {
    const got = marqueePick({ x: 100, y: 30, width: 100, height: 80 }, [fan, cpu]);
    expect(got).toEqual([
      { kind: "zone", index: 0 },
      { kind: "landmark", index: 1 },
    ]);
  });

  it("takes the case only when the band holds all of it", () => {
    const some = marqueePick({ x: 90, y: 30, width: 130, height: 80 }, [fan, cpu, c]);
    expect(some).toEqual([
      { kind: "zone", index: 0 },
      { kind: "landmark", index: 1 },
    ]);
    const all = marqueePick({ x: 90, y: 20, width: 140, height: 140 }, [fan, cpu, c]);
    expect(all).toContainEqual({ kind: "landmark", index: 2 });
    expect(all).toHaveLength(3);
  });

  it("picks nothing from an empty band", () => {
    expect(marqueePick({ x: 0, y: 0, width: 10, height: 10 }, [fan, cpu, c])).toEqual([]);
  });
});

describe("dragThresholdPx", () => {
  it("is a few pixels for a mouse and more for a finger", () => {
    expect(dragThresholdPx("mouse")).toBeGreaterThanOrEqual(3);
    expect(dragThresholdPx("mouse")).toBeLessThanOrEqual(6);
    expect(dragThresholdPx("touch")).toBeGreaterThan(dragThresholdPx("mouse"));
  });
});
