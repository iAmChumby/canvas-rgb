// The Effects gallery scrolls inside its own pane at every desktop width (#132).
// jsdom does no layout, so this test checks the stylesheet's intent, in any rule order. The layout
// itself is proven by a headless-Chromium measurement at 721, 736, 752, 760 and 1440 px
// (.build/screens/132/gallery-overlap.mjs, gitignored: it lives in the main checkout).
//
// The mistake: `@container page (max-width: 640px)` set the gallery to overflow: visible, as if the
// document scrolled. A desktop shell from 721 to about 752 px is that narrow with `.content` still
// clipped (R1), so the single-column gallery spilled its last rows over the inspector's row.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(process.cwd(), "src/styles/app.css"), "utf8")
  .replace(/\r\n/g, "\n")
  .replace(/\/\*[\s\S]*?\*\//g, "");

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
    const head = text.slice(i, open).replace(/^[\s\S]*;/, "").replace(/\s+/g, " ").trim();
    const body = text.slice(open + 1, j - 1);
    if (head.startsWith("@")) out.push(...rules(body, `${at} ${head}`.trim()));
    else out.push({ at, selectors: head.split(",").map((s) => s.trim()), body });
    i = j;
  }
  return out;
}

const GALLERY = ".rd-effects .rd-eff-gallery";
const all = rules(css).filter((r) => r.selectors.includes(GALLERY));
const overflow = (r: Rule) => /(^|[;\s])overflow(-y)?\s*:\s*([^;]+)/.exec(r.body)?.[3]?.trim();

describe("Effects gallery pane (#132)", () => {
  it("scrolls by default (overflow: auto at top level)", () => {
    expect(all.some((r) => r.at === "" && overflow(r) === "auto")).toBe(true);
  });

  it("no @container rule gives the gallery overflow: visible", () => {
    // A container query keys on the page's width, which is narrower than the window (icon rail
    // plus padding): it fires on a desktop shell where the document does not scroll.
    const offenders = all.filter((r) => r.at.startsWith("@container") && overflow(r) === "visible");
    expect(offenders.map((r) => r.at)).toEqual([]);
  });

  it("no rule at all gives the gallery overflow: visible (a phone renders .effect-gallery-wrap instead)", () => {
    expect(all.filter((r) => overflow(r) === "visible").map((r) => r.at)).toEqual([]);
  });
});
