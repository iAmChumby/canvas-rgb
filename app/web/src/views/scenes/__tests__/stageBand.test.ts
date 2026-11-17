// #378: the stage's band and the sheet's scroll padding read one number, so a padding that
// drifts away from the band cannot hide a control that took the focus under it. jsdom does no
// layout, so — like the motion test (#187) — this reads the stylesheet and checks the rules
// name the same variable and that the number is the arithmetic of the band's own rules.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");
const css = read("src/views/scenes/scenes.module.css");
const tokens = read("src/styles/tokens.css");

/** The rules below 720 px: the sheet is the whole window on a phone. */
const phone = css.slice(css.lastIndexOf("@media (max-width: 720px)"));

/** The exact `{ ... }` block of one selector (no surrounding rules). */
function blockOf(source: string, selector: string): string {
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`${esc}[^\\{]*\\{[^}]*\\}`, "s").exec(source)?.[0] ?? "";
}

/** A custom property's value in px, out of a rule block. */
function px(block: string, name: string): number {
  const found = new RegExp(`${name}:\\s*(\\d+)px`).exec(block)?.[1];
  if (found === undefined) throw new Error(`${name} is not in the block`);
  return Number(found);
}

/** A token's value in px, out of tokens.css. */
function tokenPx(name: string): number {
  const found = new RegExp(`${name}:\\s*(\\d+)px`).exec(tokens)?.[1];
  if (found === undefined) throw new Error(`${name} is not in tokens.css`);
  return Number(found);
}

/**
 * The band's height, from the band's own rules.
 *
 * The picture column: the picture, the gap and the caption. The side: the effect row, the
 * gap, the mode control's row, the gap and the note. The band: its own padding over the
 * taller of the two.
 */
function bandHeight(modeRow: number): number {
  const caption = /line-height:\s*([\d.]+)/.exec(blockOf(css, ".stageCaption"))?.[1];
  if (caption === undefined) throw new Error("the caption has no line-height");
  const column = px(blockOf(css, ".stagePicture"), "height") + tokenPx("--s1") + tokenPx("--t14") * Number(caption);
  const side =
    tokenPx("--control-h") +
    tokenPx("--s3") +
    modeRow +
    tokenPx("--s2") +
    px(blockOf(css, ".stageAway"), "min-height");
  return Math.ceil(Math.max(column, side)) + tokenPx("--s3") * 2;
}

describe("the stage's band and the sheet's pane (#378, #469)", () => {
  it("the band and the pane are two boxes, and only the pane scrolls", () => {
    expect(blockOf(css, ".stage")).toMatch(/min-height: var\(--stage-h\)/);
    // The box that holds the band and the pane does not scroll (#469), so nothing in the pane
    // can be drawn in the band's pixels ...
    expect(blockOf(css, ".sheetBody")).toMatch(/overflow: hidden/);
    expect(blockOf(css, ".sheetBody")).not.toMatch(/overflow-y: auto/);
    // ... and the pane inside it does.
    expect(blockOf(css, ".sheetScroll")).toMatch(/overflow-y: auto/);
  });

  it("nothing is pinned over the pane, so there is no scroll padding to keep in step", () => {
    expect(blockOf(css, ".stage")).not.toMatch(/position: sticky/);
    // Only the sheet's own rules matter here: another area may pin a header and pad its scroll.
    const sheetRules = [".sheet", ".sheetHead", ".sheetBody", ".sheetScroll", ".stage"]
      .map((selector) => blockOf(css, selector) + blockOf(phone, selector))
      .join("\n");
    expect(sheetRules).not.toMatch(/scroll-padding-top/);
  });

  it("the band's height is the arithmetic of the band's own rules", () => {
    const body = blockOf(css, ".sheetBody");
    // The mode control's row: one row of segments (26 + 2 x 2 + 2 x 1).
    expect(px(body, "--stage-mode-h")).toBe(32);
    expect(px(body, "--stage-h")).toBe(bandHeight(32));
  });

  it("a phone's band is taller, and says why", () => {
    const body = blockOf(phone, ".sheetBody");
    // Two rows of segments (26 x 2 + the gap 2 + the padding 4 + the border 2).
    expect(px(body, "--stage-mode-h")).toBe(60);
    expect(px(body, "--stage-h")).toBe(bandHeight(60));
  });

  it("the mode control's row is kept when the group has no mode", () => {
    expect(blockOf(css, ".stageModeRow")).toMatch(/height: var\(--stage-mode-h\)/);
  });
});
