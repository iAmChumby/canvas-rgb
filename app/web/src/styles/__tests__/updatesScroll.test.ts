// What's new and Settings scroll their own pane (#124). `.content` is overflow: hidden on a
// desktop window (R1), so a column taller than the window is clipped unless a pane scrolls.
// jsdom does no layout, so this test checks the stylesheet's intent, in any rule order.
// The layout itself is proven by a headless-Chromium measurement (.build/updates-scroll.mjs,
// gitignored: it lives in the main checkout), which runs at the widths around the shell's
// breakpoints.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");
const css = read("src/styles/app.css").replace(/\/\*[\s\S]*?\*\//g, "");

type Rule = { at: string; selectors: string[]; body: string };

// Every style rule, with the @-rule(s) it sits in (empty at top level).
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

const all = rules(css);
const decl = (r: Rule, prop: string) => new RegExp(String.raw`(^|[;\s])` + prop + String.raw`\s*:\s*([^;]+)`).exec(r.body)?.[2]?.trim();
const forSelector = (sel: string) => all.filter((r) => r.selectors.includes(sel));
const hasDecl = (rs: Rule[], prop: string, value: string, at?: (a: string) => boolean) =>
  rs.some((r) => (!at || at(r.at)) && decl(r, prop) === value);

const SCROLLERS = [".rd-settings .rd-settings-scroll", ".rd-updates .rd-updates-notes"];

describe("What's new and Settings scroll panes (#124)", () => {
  it("the view marks the grid and the notes column", () => {
    const view = read("src/views/UpdatesView.tsx");
    expect(view).toContain("rd-updates-grid");
    expect(view).toContain("rd-updates-notes");
  });

  it.each(SCROLLERS)("%s scrolls by default, on any desktop shell width", (sel) => {
    expect(hasDecl(forSelector(sel), "overflow-y", "auto", (a) => a === "")).toBe(true);
  });

  it("no rule inside @container page (min-width ...) sets overflow (the class fix, #124)", () => {
    // The page container is narrower than the window (icon rail + padding), so a scroll rule
    // keyed on it and the 720 px viewport phone fallback leave a band (721 to ~752 px) with
    // neither. A pane scrolls by default and the phone block hands scrolling to the document.
    const offenders: string[] = [];
    for (const r of all) {
      if (!/@container page \([^)]*min-width/.test(r.at)) continue;
      if (/(^|[;\s])overflow(-[xy])?\s*:/.test(r.body)) offenders.push(`${r.at} ${r.selectors.join(", ")}`);
    }
    expect(offenders).toEqual([]);
  });

  it.each([".rd-peripherals .peripheral-detail", ".rd-effects .rd-eff-inspector"])(
    "%s scrolls by default and the phone block hands it back",
    (sel) => {
      expect(hasDecl(forSelector(sel), "overflow-y", "auto", (a) => a === "")).toBe(true);
      const phone = all.filter((r) => /^@media \(max-width: 720px\)$/.test(r.at) && r.selectors.includes(sel));
      expect(hasDecl(phone, "overflow", "visible")).toBe(true);
    },
  );

  it("the phone media query hands scrolling back to the document", () => {
    const phone = all.filter((r) => /^@media \(max-width: 720px\)$/.test(r.at));
    for (const sel of [".rd-settings .rd-settings-scroll", ".rd-updates .rd-updates-grid"]) {
      expect(hasDecl(phone.filter((r) => r.selectors.includes(sel)), "overflow", "visible")).toBe(true);
    }
  });

  it("in one column the grid is the pane and the notes column stops scrolling", () => {
    const narrow = all.filter((r) => /^@media \(max-width: 960px\)$/.test(r.at));
    expect(hasDecl(narrow.filter((r) => r.selectors.includes(".rd-updates .rd-updates-grid")), "overflow-y", "auto")).toBe(true);
    expect(hasDecl(narrow.filter((r) => r.selectors.includes(".rd-updates .rd-updates-notes")), "overflow", "visible")).toBe(true);
  });
});
