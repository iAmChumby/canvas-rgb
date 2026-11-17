// The rows of the Scenes page and the menu of a group: the rules that hold them in line.
//
// jsdom does no layout, so these tests read the stylesheet and ask the question the eye asks:
// does the row's grid give the Unassigned row the same cells the group rows have, and does the
// menu line every item up, the link ("Edit effect") as well as the keys? The pixels are measured
// in a real browser (.build/screens/384/r2/, gitignored: it lives in the main checkout).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(process.cwd(), "src/views/scenes/scenes.module.css"), "utf8")
  .replace(/\r\n/g, "\n")
  .replace(/\/\*[\s\S]*?\*\//g, "");

type Rule = { selectors: string[]; body: string };

/** Every rule in the sheet. The rules inside an at-rule are read on their own. */
function rules(text: string): Rule[] {
  const out: Rule[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const head = m[1];
    const body = m[2];
    if (head === undefined || body === undefined) continue;
    const selectors = head
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    if (selectors.length > 0) out.push({ selectors, body });
  }
  return out;
}

const all = rules(css);

/** The rule that carries the class, the first one that does. */
function bodyOf(selector: string): string {
  const rule = all.find((r) => r.selectors.includes(selector));
  if (!rule) throw new Error(`the sheet has no rule for ${selector}`);
  return rule.body;
}

/** The rules whose selector starts with the class and a child. */
function childRules(selector: string): Rule[] {
  return all.filter((r) => r.selectors.some((s) => s.startsWith(`${selector} >`)));
}

/** The value of a property in a rule's body. */
function value(body: string, property: string): string {
  const m = body.match(new RegExp(`${property}\\s*:\\s*([^;]+)`));
  if (!m || m[1] === undefined) throw new Error(`the rule has no ${property}`);
  return m[1].trim();
}

/** A list of values, split at the spaces that are not inside brackets. */
function splitTop(list: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let current = "";
  for (const ch of list) {
    if (ch === "(") depth++;
    if (ch === ")") depth--;
    if (/\s/.test(ch) && depth === 0) {
      if (current) out.push(current);
      current = "";
      continue;
    }
    current += ch;
  }
  if (current) out.push(current);
  return out;
}

describe("the rows of the Scenes page", () => {
  it("every row has a column for the handle, so the rows without one keep their place", () => {
    // The row's grid: the handle's column, the name's, the run's, and the three controls'.
    const columns = splitTop(value(bodyOf(".row"), "grid-template-columns"));
    expect(columns).toHaveLength(6);
    expect(columns[0]).toBe("28px");
    // The narrow row has the same first column.
    const narrow = all.find(
      (r) => r.selectors.includes(".row") && r.body.includes("grid-template-columns: 28px minmax(120px"),
    );
    expect(narrow).toBeTruthy();
  });

  it("the Unassigned row's dark mark stands in the effect's column, the fourth of the grid", () => {
    const columns = splitTop(value(bodyOf(".row"), "grid-template-columns"));
    // The group row's cells: the handle, the name, the run, then the effect, the mode and the
    // fader. The Unassigned row has no effect, so its dark mark stands where the effect's key
    // stands.
    expect(columns).toHaveLength(6);
    const dark = bodyOf(".dark");
    expect(value(dark, "grid-column")).toBe("4 / -1");
  });
});

describe("the menu of a group", () => {
  it("lines every item up: the keys and the link alike", () => {
    // The link ("Edit effect") is an anchor and the rest are keys: a rule that names only
    // `button` leaves the link in the middle of the panel, out of line with the rest.
    const aligned = childRules(".sheetMenu").filter((r) => r.body.includes("justify-content: flex-start"));
    expect(aligned.length).toBeGreaterThan(0);
    for (const rule of aligned) {
      expect(rule.selectors.some((s) => s.includes("button"))).toBe(true);
      expect(rule.selectors.some((s) => s.includes("a"))).toBe(true);
    }
  });
});

// #469: at a narrow width the mode cell sits past the fader, and all three stay on ONE line. The
// pinned columns alone put the fader in a second grid row (measured: 133 px rows became 165 px).
describe("the narrow row's controls (#469)", () => {
  it("pins the fader and the mode to the first grid row", () => {
    const all = rules(css);
    for (const sel of [".controls .fader", ".controls .mode"]) {
      const pinned = all.filter((r) => r.selectors.includes(sel));
      expect(pinned.length, `${sel} has a rule`).toBeGreaterThan(0);
      expect(pinned.some((r) => /grid-row:\s*1\s*;/.test(r.body)), `${sel} sits on row 1`).toBe(true);
    }
  });

  it("asks the effect picker to stay clear of its own row", () => {
    // jsdom does no layout: the placement rule itself is pinned in PopoverPlacement.test.ts; this
    // pins that the row asks for it, so the picker never covers the mode cell (#469).
    const row = readFileSync(resolve(process.cwd(), "src/views/scenes/GroupRow.tsx"), "utf8");
    const picker = row.slice(row.indexOf("label={`What ${group.name} plays`}"));
    expect(picker.slice(0, 600)).toContain("keepClearOfAnchor");
  });
});

