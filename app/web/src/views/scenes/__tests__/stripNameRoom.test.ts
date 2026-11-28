// The scene's name keeps the phone strip (#422).
//
// At 390 wide the strip read "Eveni..." beside the amber "Dark" mark. The row is one line:
// the page's title, what is playing (the scene's name), the state mark, the page's keys and the
// power control. jsdom does no layout, so this test checks the stylesheet's rule: a phone keeps
// the strip's own rule for the mark (its icon only, the words in its tooltip), and the Scenes
// page must not put the words back, because they took the room the name needed.
//
// The layout itself is measured at all six sizes in both themes by a headless-Chromium script
// (.build/screens/422/measure.mjs, gitignored: it lives in the main checkout): the name is never
// ellipsised, and the strip's height and the page's top are equal with and without the mark.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");
const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");

type Rule = { at: string; selectors: string[]; body: string };

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

const decl = (body: string, prop: string) =>
  new RegExp(String.raw`(^|[;\s])` + prop + String.raw`\s*:\s*([^;]+)`).exec(body)?.[2]?.trim();

const appRules = rules(strip(read("src/styles/app.css")));
const sceneRules = rules(strip(read("src/views/scenes/scenes.module.css")));
const phone = (r: Rule) => /@media \(max-width: 720px\)/.test(r.at);
// #586 R4: the mark's words now follow the strip's own content box, not the window's width.
const narrowStrip = (r: Rule) => /@container strip \(max-width: 1150px\)/.test(r.at);

describe("the phone strip keeps room for the scene's name (#422)", () => {
  it("the strip's own narrow rule hides the state mark's words on every page", () => {
    const rule = appRules.find((r) => narrowStrip(r) && r.selectors.includes(".strip-mark-text"));
    expect(decl(rule?.body ?? "", "display")).toBe("none");
  });

  it("the Scenes page does not put the mark's words back on a phone", () => {
    // On origin/main a rule here made the mark's words "1 group dark" take the row at phone
    // width, and "Evening" was cut to "Eveni...". The mark is its icon; its label and its
    // details panel name the fault.
    const offenders = sceneRules
      .filter((r) => phone(r))
      .filter((r) => r.selectors.some((s) => s.includes(".strip-mark-text")))
      .filter((r) => {
        const shown = decl(r.body, "display");
        return shown !== undefined && shown !== "none";
      })
      .map((r) => `${r.at} ${r.selectors.join(", ")}`);
    expect(offenders).toEqual([]);
  });

  it("nothing caps the scene's name below the row's own room", () => {
    // A phone's name is the strip's `.strip-effect-name`, which the page must not give a width
    // or a cap.
    const caps = sceneRules
      .filter((r) => r.selectors.some((s) => s.includes(".strip-effect-name")))
      .filter((r) => decl(r.body, "max-width") !== undefined || decl(r.body, "width") !== undefined);
    expect(caps.map((r) => `${r.at} ${r.selectors.join(", ")}`)).toEqual([]);
  });
});
