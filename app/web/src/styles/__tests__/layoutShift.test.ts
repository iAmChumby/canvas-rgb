// A control keeps one size when its content changes (#367, #369).
//
// Two bugs, one cause: a box whose size comes from its content moves whenever that content
// changes. The Scenes effect picker is a panel as wide as its content, and the line under its
// list says what the lights show - so pointing at a row moved the whole menu. The colour picker's
// hex code has no width of its own, and the picker grew and shrank as the code changed.
//
// jsdom does no layout, so this test checks the stylesheet's intent, in any rule order: the width
// a box is given, and the tracks a grid row is given. The layout itself is proven by a headless
// Chromium measurement (.build/screens/367-369/measure-367-369.mjs, gitignored: it lives in the
// main checkout), which measures the menu and the swatch in every state.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");

/** Every stylesheet of the editor, so the guards below look at all of them. */
function cssFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(resolve(process.cwd(), dir))) {
    const full = join(dir, name);
    if (statSync(resolve(process.cwd(), full)).isDirectory()) out.push(...cssFiles(full));
    else if (name.endsWith(".css")) out.push(full);
  }
  return out;
}

type Rule = { at: string; selectors: string[]; body: string };

/** Every style rule, with the @-rule(s) it sits in (empty at top level). */
function rules(text: string, at = ""): Rule[] {
  const out: Rule[] = [];
  let i = 0;
  while (i < text.length) {
    const open = text.indexOf("{", i);
    if (open === -1) break;
    let depth = 1;
    let j = open + 1;
    while (j < text.length && depth > 0) {
      if (text[j] === "{") depth += 1;
      else if (text[j] === "}") depth -= 1;
      j += 1;
    }
    // A statement at-rule (`@import ...;`) ends in `;` and has no block: skip past it.
    const head = text.slice(i, open).replace(/^[\s\S]*;/, "").replace(/\s+/g, " ").trim();
    const body = text.slice(open + 1, j - 1);
    if (head.startsWith("@")) {
      out.push(...rules(body, `${at} ${head}`.trim()));
    } else {
      out.push({ at, selectors: head.split(",").map((s) => s.trim()), body });
    }
    i = j;
  }
  return out;
}

const appCss = rules(read("src/styles/app.css").replace(/\/\*[\s\S]*?\*\//g, ""));
const scenesCss = rules(read("src/views/scenes/scenes.module.css").replace(/\/\*[\s\S]*?\*\//g, ""));
const motionCss = rules(read("src/styles/motion.css").replace(/\/\*[\s\S]*?\*\//g, ""));

const decl = (r: Rule, prop: string) =>
  new RegExp(String.raw`(^|[;\s])` + prop + String.raw`\s*:\s*([^;]+)`).exec(r.body)?.[2]?.trim();
const forSelector = (list: Rule[], sel: string) => list.filter((r) => r.selectors.includes(sel));
const hasDecl = (list: Rule[], prop: string, value?: string) =>
  list.some((r) => (value === undefined ? decl(r, prop) !== undefined : decl(r, prop) === value));

describe("the Scenes effect picker keeps one width (#367)", () => {
  it("the row's PLAYING column is a fixed track, not one that grows with the tag", () => {
    // The tag is drawn only on the row the group plays, and the preview moves it to the row
    // under the pointer. An `auto` track gave the tag the room it needed and the row grew.
    const row = forSelector(scenesCss, ".pickRow");
    expect(row.length).toBeGreaterThan(0);
    for (const r of row) {
      const columns = decl(r, "grid-template-columns");
      expect(columns, `.pickRow in ${r.at || "the sheet"}`).toBeDefined();
      expect(columns, `.pickRow in ${r.at || "the sheet"} ends with an auto track`).not.toMatch(/\bauto$/);
      expect(columns, `.pickRow in ${r.at || "the sheet"} ends with a fixed track`).toMatch(/\d+px$/);
    }
  });

  it("the menu is as wide as the design says, not as wide as its content", () => {
    // The panel is as wide as its content, so the line under the list - which says what the
    // lights show - moved the whole menu. The width is set in the stylesheet instead.
    const popover = forSelector(scenesCss, ".pickerPopover");
    expect(popover.length).toBeGreaterThan(0);
    for (const r of popover) {
      expect(decl(r, "width"), ".pickerPopover has no width").toBeDefined();
      expect(decl(r, "max-width"), ".pickerPopover has no max-width").toBeDefined();
    }
  });
});

describe("the colour picker keeps one size (#369)", () => {
  it("the swatch beside the hex code never gives up its size", () => {
    expect(hasDecl(forSelector(appCss, ".color-swatch"), "flex", "0 0 auto")).toBe(true);
    expect(hasDecl(forSelector(appCss, ".picker-preview"), "flex", "0 0 auto")).toBe(true);
  });

  it("the hex code has a width of its own and tabular digits", () => {
    expect(hasDecl(forSelector(appCss, ".color-hex"), "width", "7ch")).toBe(true);
    expect(hasDecl(forSelector(appCss, ".color-hex"), "font-variant-numeric", "tabular-nums")).toBe(true);
    // The field in the picker never gets narrower than the code it holds.
    expect(hasDecl(forSelector(appCss, ".picker-hex"), "min-width", "8ch")).toBe(true);
  });

  it("the quick swatches keep their place: the one that matches is marked, not dropped", () => {
    expect(hasDecl(forSelector(appCss, ".picker-swatch"), "width", "22px")).toBe(true);
    expect(hasDecl(forSelector(appCss, ".picker-swatch"), "height", "22px")).toBe(true);
    // The colour in the field is marked with a ring, so the list is never a different length.
    expect(forSelector(appCss, '.picker-swatch[aria-current="true"]').length).toBeGreaterThan(0);
    const picker = read("src/components/ColorPicker.tsx");
    expect(picker).toContain('aria-current={current ? "true" : undefined}');
    const controls = read("src/views/PropertyControls.tsx");
    expect(controls).not.toMatch(/parsed === hex\.toLowerCase\(\)/);
  });
});

describe("the class guard: a menu or a figure that changes must not change a box", () => {
  it("no hover, focus or active rule changes the size of a box", () => {
    // The class rule: hover-only content must not take space. A rule that changes a box's size
    // when the pointer arrives moves everything under it.
    const sizeProps =
      "(^|[;\\s])(width|height|min-width|min-height|max-width|max-height|padding|padding-top|padding-bottom|padding-left|padding-right|margin|margin-top|margin-bottom|margin-left|margin-right|border-width|font-size|gap|row-gap|column-gap|letter-spacing|flex-basis)\\s*:";
    const offenders: string[] = [];
    for (const file of cssFiles("src")) {
      for (const r of rules(read(file).replace(/\/\*[\s\S]*?\*\//g, ""))) {
        if (!/:(hover|focus|focus-visible|active)/.test(r.selectors.join(","))) continue;
        if (new RegExp(sizeProps).test(r.body)) {
          offenders.push(`${file} ${r.selectors.join(", ")} { ${r.body.replace(/\s+/g, " ").trim()} }`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it("every panel whose content is live has one width", () => {
    // A panel as wide as its content moves whenever that content changes. These panels hold
    // readings and figures that change while they are open, so their width is set.
    for (const [list, sel] of [
      [appCss, ".strip-details"],
      [appCss, ".audio-popover"],
      [motionCss, ".fan-bulk-popover"],
      [scenesCss, ".pickerPopover"],
    ] as [Rule[], string][]) {
      const found = forSelector(list, sel);
      expect(found.length, `${sel} has no rule`).toBeGreaterThan(0);
      for (const r of found) expect(decl(r, "width"), `${sel} has no width`).toBeDefined();
    }
  });
});
