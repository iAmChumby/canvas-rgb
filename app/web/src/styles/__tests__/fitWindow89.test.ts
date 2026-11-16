// Fit the window at the shell/container seam (#89). A window 721-772 px wide has a desktop shell
// but a page container at or under the width the device head needs to keep its Access switches on
// one line. jsdom does no layout: these tests pin the stylesheet's intent (the wrap threshold and
// the base wrap guard), in any rule order, the way effectsGallery.test.ts and updatesScroll.test.ts
// do. A CSS-text test cannot prove anything actually fits — that is what the headless-Chromium sweep
// over every window width 640-800 does (.build/screens/89/sweep.mjs, gitignored in the main checkout).
// Before the follow-up fix the Read switch was cut 21 px at 752 px and up to 20 px at 753-772 px.

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
// The head needs about 661 px of page width, so wrap from 660 px down (a 772 px window and below).
// 640 px was too low: it left 753-772 px windows clipping the switch. The sweep proves the fit.
const PAGE_PHONE = "@container page (max-width: 660px)";

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

describe("Peripherals at the 660 px page edge (#89)", () => {
  it("wraps the device head whenever the page is 660 px or less on a desktop shell", () => {
    const head = forSelector(".rd-peripherals .peripheral-head");
    expect(head.some((r) => r.at === PAGE_PHONE && decl(r, "flex-wrap") === "wrap")).toBe(true);
    // 640 px left 753-772 px windows clipping the switch: the rule must not be narrower than 660.
    expect(head.some((r) => r.at === "@container page (max-width: 640px)" && decl(r, "flex-wrap") === "wrap")).toBe(false);
  });

  it("puts the Access switches on their own row at that edge, like the 639 px phone block", () => {
    const access = forSelector(".rd-peripherals .rd-peri-access");
    const atEdge = access.find((r) => r.at === PAGE_PHONE && decl(r, "flex") === "1 1 100%");
    expect(atEdge).toBeDefined();
    // Page 639 and page 640 must look the same: keep the phone block's gap and alignment.
    expect(decl(atEdge!, "gap")).toBe("var(--s3)");
    expect(decl(atEdge!, "justify-content")).toBe("flex-end");
  });

  it("lets the base head wrap so Access drops to its own line at any width it stops fitting", () => {
    // This keeps wide windows identical (flex-wrap never triggers when everything fits).
    const base = forSelector(".peripheral-head").filter((r) => r.at === "" && decl(r, "display") === "flex");
    expect(base.some((r) => decl(r, "flex-wrap") === "wrap")).toBe(true);
  });
});
