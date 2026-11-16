// Fit the window at the shell/container seam (#89). A window 721-752 px wide has a desktop shell
// but a page container at or under the phone edge. jsdom does no layout, so these tests check the
// stylesheet's intent, in any rule order, the way effectsGallery.test.ts and updatesScroll.test.ts
// do. The layout itself is proven by a headless-Chromium measurement (.build/screens/89, gitignored
// in the main checkout): before the fix Devices was cut 766 px and Peripherals' Read switch 21 px.

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

const all = rules(css);
const decl = (r: Rule, prop: string) =>
  new RegExp(String.raw`(^|[;\s])` + prop + String.raw`\s*:\s*([^;]+)`).exec(r.body)?.[2]?.trim();
const forSelector = (sel: string) => all.filter((r) => r.selectors.includes(sel));
const PHONE = "@media (max-width: 720px)";
const PAGE_PHONE = "@container page (max-width: 640px)";

describe("Devices at the 721-752 px seam (#89)", () => {
  it("keeps the list's own scroller on a desktop shell, and hands it back only on a phone", () => {
    const list = forSelector(".rd-devices .rd-dev-list");
    // The pane scrolls by default, whatever the page container's width.
    expect(list.some((r) => r.at === "" && decl(r, "overflow") === "auto")).toBe(true);
    // A @container rule also fires on a desktop shell at 721-752 px, where .content is clipped:
    // giving the list overflow: visible or flex: 0 0 auto there left the rows cut off.
    expect(
      list
        .filter((r) => r.at.startsWith("@container") && (decl(r, "overflow") === "visible" || decl(r, "flex") === "0 0 auto"))
        .map((r) => r.at),
    ).toEqual([]);
    expect(list.some((r) => r.at === PHONE && decl(r, "overflow") === "visible")).toBe(true);
  });

  it("keeps the load frame flex on a desktop shell, and lets it grow only on a phone", () => {
    const frame = forSelector('.rd-devices .load-frame[data-state="ready"]');
    expect(
      frame.filter((r) => r.at.startsWith("@container") && decl(r, "flex") === "0 0 auto").map((r) => r.at),
    ).toEqual([]);
    expect(frame.some((r) => r.at === PHONE && decl(r, "flex") === "0 0 auto")).toBe(true);
  });
});

describe("Peripherals at the 640 px page edge (#89)", () => {
  it("wraps the device head when the page is at the phone edge on a desktop shell", () => {
    const head = forSelector(".rd-peripherals .peripheral-head");
    expect(head.some((r) => r.at === PAGE_PHONE && decl(r, "flex-wrap") === "wrap")).toBe(true);
  });

  it("puts the Access switches on their own row at that edge", () => {
    const access = forSelector(".rd-peripherals .rd-peri-access");
    expect(access.some((r) => r.at === PAGE_PHONE && decl(r, "flex") === "1 1 100%")).toBe(true);
  });
});
