// The drag session's rules (#257 phase 2): the words the live region says, the keyboard's target
// cycle, and the one reorder rule a drop uses. No DOM: the sensors' decisions are pure.

import { describe, expect, it } from "vitest";
import { DEVICES } from "@/lib/fixtures";
import { canvasWith } from "./groupCanvasFixture";
import {
  CANCEL_ANNOUNCEMENT,
  cycleTarget,
  dropAnnouncement,
  liftAnnouncement,
  lifted,
  liftsOnHold,
  liftsOnMove,
  pressMoved,
  reorderToSlot,
  stackSlots,
  stageTargets,
  targetAnnouncement,
  targetKey,
} from "@/lib/dragSession";
import type { StackLayer } from "@/proto/StackLayer";

const layerOf = (id: string, name: string): StackLayer => ({
  id,
  name,
  effect: "solid",
  props: {},
  area: { kind: "canvas" },
  blend: "normal",
  opacity: 100,
  visible: true,
});

const layers = [layerOf("l1", "Sunset"), layerOf("l2", "Heat"), layerOf("l3", "Sparkles")];
const canvas = canvasWith([
  { id: "royuan:3151:4015#z0", label: "ROYUAN MonsGeek Keyboard · Zone 1", device: "royuan:3151:4015", x: 0, y: 0, width: 100, height: 50 },
  { id: "openrgb:mouse-1#z0", label: "Mouse · Zone 1", device: "openrgb:mouse-1", x: 100, y: 0, width: 100, height: 50 },
]);
const names = { canvas, devices: DEVICES, layers };
const effect = { kind: "effect" as const, effect: "fire", name: "Fire" };
const layer = { kind: "layer" as const, id: "l2", name: "Heat" };

describe("the live region", () => {
  it("says how to move a lifted effect, then where it would land", () => {
    expect(liftAnnouncement(effect)).toBe(
      "Lifted Fire. Left and Right pick where it goes, Up and Down pick a place in the list; Enter drops it, Escape cancels.",
    );
    expect(targetAnnouncement({ kind: "canvas" }, names)).toBe("Whole canvas.");
    expect(targetAnnouncement({ kind: "device", device: "royuan:3151:4015" }, names)).toBe(
      "ROYUAN MonsGeek Keyboard.",
    );
    expect(dropAnnouncement(effect, { kind: "stackSlot", at: 3 }, names)).toBe("Fire added to the top of the list.");
    expect(dropAnnouncement(effect, { kind: "stackSlot", at: 2 }, names)).toBe("Fire added above Heat.");
    expect(dropAnnouncement(effect, { kind: "zone", device: "royuan:3151:4015", zone: 0 }, names)).toBe(
      "Fire added to ROYUAN MonsGeek Keyboard · Zone 1.",
    );
  });

  it("says how to move a lifted layer, and that a cancel changed nothing", () => {
    expect(liftAnnouncement(layer)).toBe(
      "Lifted Heat. Up and Down pick a place in the list; Enter drops it, Escape cancels.",
    );
    expect(dropAnnouncement(layer, { kind: "stackSlot", at: 3 }, names)).toBe("Heat moved to the top of the list.");
    expect(CANCEL_ANNOUNCEMENT).toBe("Cancelled; nothing changed.");
  });
});

describe("the keyboard's targets", () => {
  it("cycles the whole canvas, each device in the group, then a new rectangle", () => {
    const targets = stageTargets(canvas);
    expect(targets[0]).toEqual({ kind: "canvas" });
    expect(targets[1]).toEqual({ kind: "device", device: "royuan:3151:4015" });
    expect(targets.at(-1)).toEqual({ kind: "rect", x: 100, y: 60, width: 120, height: 80 });
    // The devices are each offered once, however many lights they have.
    const devices = targets.filter((target) => target.kind === "device");
    expect(new Set(devices.map((target) => (target.kind === "device" ? target.device : ""))).size).toBe(devices.length);
    expect(devices).toHaveLength(2);
  });

  it("moves one place and wraps at both ends", () => {
    const targets = stageTargets(canvas);
    const first = cycleTarget(targets, null, 1);
    const second = cycleTarget(targets, first, 1);
    expect(targetKey(first)).toBe("canvas");
    expect(targetKey(second)).toBe("device:royuan:3151:4015");
    expect(targetKey(cycleTarget(targets, first, -1))).toBe(targetKey(targets.at(-1) ?? null));
    expect(targetKey(cycleTarget(targets, targets.at(-1) ?? null, 1))).toBe("canvas");
  });

  it("offers every insertion point in the stack, bottom first", () => {
    const slots = stackSlots(3);
    expect(slots).toHaveLength(4);
    expect(slots[0]).toEqual({ kind: "stackSlot", at: 0 });
    expect(slots[3]).toEqual({ kind: "stackSlot", at: 3 });
  });
});

describe("a layer moved to an insertion point", () => {
  const order = (list: StackLayer[]) => list.map((entry) => entry.id);

  it("lands above the layer it was dropped over, counting before it was lifted", () => {
    // Sunset is at the bottom; slot 3 is the top, so it passes both others.
    expect(order(reorderToSlot(layers, "l1", 3))).toEqual(["l2", "l3", "l1"]);
    // Sparkles is at the top; slot 0 is the bottom.
    expect(order(reorderToSlot(layers, "l3", 0))).toEqual(["l3", "l1", "l2"]);
    // Heat between Sunset and Sparkles: dropping it back where it is changes nothing.
    expect(order(reorderToSlot(layers, "l2", 2))).toEqual(["l1", "l2", "l3"]);
  });
});

describe("the press sensor", () => {
  it("lifts a mouse or a pen on the move, and a finger on the hold", () => {
    expect(pressMoved({ x: 10, y: 10 }, { x: 15, y: 10 })).toBe(true);
    expect(pressMoved({ x: 10, y: 10 }, { x: 13, y: 12 })).toBe(false);
    expect(liftsOnMove("mouse")).toBe(true);
    expect(liftsOnMove("pen")).toBe(true);
    expect(liftsOnMove("touch")).toBe(false);
    expect(liftsOnHold("touch")).toBe(true);
    expect(liftsOnHold("mouse")).toBe(false);
  });

  it("starts with nothing lifted", () => {
    expect(lifted(effect, "keyboard")).toEqual({
      source: effect,
      mode: "keyboard",
      target: null,
      point: null,
      hint: null,
    });
  });
});
