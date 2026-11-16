// The held key and the focused key are two different looks (#375).
//
// The plan's keys shared one rule: a key that was held down and a key that had the keyboard's
// focus were drawn the same way, so a key with the focus on it read as a pressed one. The rule
// is split: the focused key keeps the soft fill, and the held key takes a solid fill of the
// live hue with its legend in the ink that goes with that fill.
//
// jsdom does no layout and no colours, so this test reads the stylesheet's intent: the two
// rules are apart, the held key's fill and legend are tokens, and the token holds its contrast
// in both themes (the numbers are in the report: 5.8:1 in the dark theme, 5.2:1 in the light
// one, at every hue the live preview can take).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");

type Rule = { selectors: string[]; body: string };

/** Every style rule of a stylesheet, with the comments taken out. */
function rules(text: string): Rule[] {
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
    const head = text.slice(i, open).replace(/^[\s\S]*;/, "").replace(/\s+/g, " ").trim();
    const body = text.slice(open + 1, j - 1);
    if (head.startsWith("@")) out.push(...rules(body));
    else out.push({ selectors: head.split(",").map((s) => s.trim()), body });
    i = j;
  }
  return out;
}

const app = rules(read("src/styles/app.css").replace(/\/\*[\s\S]*?\*\//g, ""));
const tokens = read("src/styles/tokens.css");

/**
 * The rule that carries a selector, if there is one.
 *
 * The selector has to be the whole one: `.pad-readouts` carries `.pad-readout` as its opening
 * letters, and a substring match hands back that rule instead of the readout's own.
 */
function ruleFor(selector: string): Rule | undefined {
  return app.find((rule) => rule.selectors.includes(selector));
}

describe("the held key and the focused key", () => {
  it("are two rules, not one", () => {
    const pressed = app.filter((rule) => rule.selectors.some((s) => s.includes('[data-pressed="true"]')));
    expect(pressed.length).toBeGreaterThan(0);
    for (const rule of pressed) {
      for (const selector of rule.selectors) {
        expect(selector, `a held key shares a rule with the focus: ${selector}`).not.toContain(
          ":focus-visible",
        );
      }
    }
    // The focused key keeps a rule of its own.
    const focus = ruleFor(".pad-target:focus-visible .pad-btn");
    expect(focus).toBeDefined();
  });

  it("fills the held key with a token and prints its legend in the ink that goes with it", () => {
    const fill = ruleFor('.pad-target[data-pressed="true"] .pad-btn');
    expect(fill).toBeDefined();
    expect(fill!.body).toContain("var(--live-press)");
    expect(fill!.body).toContain("var(--live)");

    const legend = ruleFor('.pad-target[data-pressed="true"] .pad-print');
    expect(legend).toBeDefined();
    expect(legend!.body).toContain("var(--on-accent)");
  });

  it("has a pressed fill in both themes", () => {
    // The dark theme's fill and the light theme's fill are both there.
    expect(tokens).toContain("--live-press: hsl(var(--live-h) 78% 72%)");
    expect(tokens).toContain("--live-press: hsl(var(--live-h) 70% 26%)");
  });

  it("draws the four paddles as ghosts", () => {
    const ghost = ruleFor('.pad-target[data-ghost="true"] .pad-btn');
    expect(ghost).toBeDefined();
    expect(ghost!.body).toContain("stroke-dasharray");
    expect(ghost!.body).toContain("fill: none");
  });
});

describe("the plan's numbers on a phone", () => {
  const css = read("src/styles/app.css").replace(/\/\*[\s\S]*?\*\//g, "");

  /** The body of the narrow-width block, from its own brace to its own closing one. */
  function narrowBlock(): string {
    const at = css.indexOf("@media (max-width: 560px)");
    expect(at).toBeGreaterThan(-1);
    const end = css.indexOf("\n}", at);
    expect(end).toBeGreaterThan(at);
    return css.slice(at, end);
  }

  it("writes the drawing's text large enough that it is read at 14 px", () => {
    const block = narrowBlock();
    // The plan is scaled down to the room it has, so the text inside it is written larger.
    expect(block).toMatch(/\.pad-travel-value \{\s*font-size: 18px;/);
    expect(block).toMatch(/\.pad-travel-label \{\s*font-size: 17px;/);
    expect(block).toMatch(/\.pad-print \{\s*font-size: 16px;/);
    // Nothing that places a box or a coordinate moves, so no panel can shift.
    const declared = [...block.matchAll(/^\s+([a-z-]+):/gm)].map((m) => m[1]);
    expect(new Set(declared)).toEqual(new Set(["font-size"]));
  });

  it("leaves the stick numbers as page text at the floor", () => {
    // The stick's numbers are not drawing units: they are page text, already at 14 px.
    const readout = ruleFor(".pad-readout");
    expect(readout).toBeDefined();
    expect(readout!.body).toContain("var(--t14)");
  });

  it("sizes the plan's legend in one rule, so no later rule can shrink it again", () => {
    // A second `.pad-print` rule further down the sheet took the size with it once: the
    // legend fell to 12 units and the plan's letters dropped under the floor.
    const prints = app.filter((rule) => rule.selectors.includes(".pad-print"));
    // The base rule and the phone's: nothing else may set this size again.
    expect(prints.length).toBe(2);
    const sizes = prints.map((rule) => Number(/font-size: (\d+)px/.exec(rule.body)?.[1] ?? "0"));
    expect(sizes.every((px) => px >= 14)).toBe(true);
    // The word at the held row's edge is read, so it too is at the floor.
    const pressed = ruleFor(".pad-map-pressed");
    expect(pressed).toBeDefined();
    expect(pressed!.body).toContain("var(--t14)");
  });

  it("holds three lines of the strip at every width, by holding the height in one rule", () => {
    // The width claim is this count: no width is measured here. The height lives in one rule
    // with no media override, so it is the same at every width: three 14 px lines at the
    // sheet's 1.45 line height, plus the 2 px between the status word and its note. A media
    // block that wrote two lines again would show up as a second rule here, and the unreadable
    // pad's note (61 characters, two lines at 390) could then push the panels below it down.
    const states = app.filter((rule) => rule.selectors.includes(".pad-state"));
    expect(states.length).toBe(1);
    expect(states[0]!.body).toMatch(/min-height: calc\(var\(--t14\) \* 3 \* 1\.45 \+ 2px\)/);
  });
});
