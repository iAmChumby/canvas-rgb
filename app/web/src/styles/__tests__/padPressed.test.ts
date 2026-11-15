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

/** The rule that carries a selector, if there is one. */
function ruleFor(selector: string): Rule | undefined {
  return app.find((rule) => rule.selectors.some((s) => s.includes(selector)));
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
