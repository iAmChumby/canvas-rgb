// #469: a tall picker was clamped back up over its anchor's row, covering the mode cell beside
// the effect key. The placement rule is a pure function so it can be checked without layout.
// GAP = 6, EDGE = 8, MIN_H = 96, the constants in components/Popover.tsx.
import { describe, expect, it } from "vitest";
import { placePanel, type AnchorRect } from "@/components/Popover";

const anchor = (top: number, bottom: number): AnchorRect => ({
  left: 70,
  right: 388,
  top,
  bottom,
  width: 318,
});

describe("a panel that must stay clear of its anchor's row (#469)", () => {
  it("hangs below the row when the panel is too tall to fit there", () => {
    // 768x1024: a 958 px picker in a 1024 px window. The old clamp put its top at 58 px, over
    // the row at 60-92 px; the fixed panel starts under the row and scrolls inside the room.
    const a = anchor(60, 92);
    const old = placePanel(a, 768, 1024, 958, 380, false);
    expect(old.top).toBeLessThan(a.bottom);

    const p = placePanel(a, 768, 1024, 958, 380, true);
    expect(p.up).toBe(false);
    expect(p.top).toBe(a.bottom + 6);
    expect(p.maxHeight).toBe(1024 - 8 - p.top);
    expect(p.top).toBeGreaterThan(a.bottom);
  });

  it("keeps the phone's picker under the row too", () => {
    const a = anchor(120, 152);
    const p = placePanel(a, 390, 844, 828, 374, true);
    expect(p.up).toBe(false);
    expect(p.top).toBe(a.bottom + 6);
    expect(p.maxHeight).toBe(844 - 8 - p.top);
    expect(p.top).toBeGreaterThan(a.bottom);
  });

  it("flips above when the row is near the window's bottom", () => {
    const a = anchor(748, 780);
    const p = placePanel(a, 1024, 800, 400, 280, true);
    expect(p.up).toBe(true);
    expect(p.top + p.maxHeight).toBeLessThanOrEqual(a.top - 6);
    expect(p.top).toBeGreaterThanOrEqual(8);
  });

  it("keeps a short panel's own size", () => {
    const a = anchor(60, 92);
    const p = placePanel(a, 1024, 900, 200, 280, true);
    expect(p.top).toBe(a.bottom + 6);
    expect(p.maxHeight).toBe(200);
  });

  it("leaves other panels' old clamping alone", () => {
    const a = anchor(60, 92);
    const p = placePanel(a, 768, 1024, 958, 380, false);
    expect(p.top).toBe(1024 - 8 - 958);
    expect(p.maxHeight).toBe(1024 - 16);
  });
});
