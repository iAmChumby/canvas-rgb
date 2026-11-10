// Controls that run past their panel or the window, measured in a real browser (issues #263, #268).
//
// jsdom has no layout, so a unit test cannot see a select that is wider than the Lighting panel
// or a "Reset to automatic" key that sticks out of its note. This serves the static export
// (app/web/out) on loopback, opens every view of the demo at the six redesign sizes in both colour
// schemes, and reports, with the element named:
//   - an element inside a box (a `.panel`, a `.rule` note, or the row containers a view lists in
//     VIEWS) whose right edge passes that box's right edge by more than 1 px;
//   - a box (a `.rule`, or a panel inside a panel) whose right edge passes the right edge of the
//     panel that holds it;
//   - any element, inside a panel or not, whose right edge passes the window's width, and any
//     `.panel` wider than the window;
//   - a view whose document scrolls sideways (scrollWidth > clientWidth);
//   - a panel that scrolls sideways itself (its content is wider than it shows);
//   - a figure's unit that has wrapped under the figure ("70" over "°C", #268).
// The slack is 1 px. Left out: an element inside a scroll container that scrolls on x (the
// container is meant to be wider than what it shows), and an element that is not drawn (display
// none, hidden, opacity 0: a tooltip before hover), is only there for a screen reader (1-2 px) or,
// for the box checks, is fixed to the window (a toast, a dialog). A `.panel` that scrolls on x is
// NOT exempt: what is inside it is measured against the panel's own right edge. (An `overflow-y:
// auto` panel computes `overflow-x: auto` too, as the Fans right panel does, which is how a
// scroll-x exemption would have hidden everything in it.)
//
// Only the topmost offender in a chain is reported: when a row overflows, its children do too,
// and listing each would bury the row.
//
// A view with nothing measured FAILS. The one exception is the list NO_PANEL below, the views
// that are known to draw no `.panel`; they list their own row containers in `boxes`, and a view in
// the list that draws none of those fails as well.
//
// Some states only exist after a click: the Fans "Don't exceed" row shows "Reset to automatic"
// once a limit is set, and the Peripherals page shows the Attack Shark X6's Lighting panel once
// the device has been read. Those are extra rows in VIEWS below, with a `prep` that gets there.
// Nothing waits a fixed time: a row is measured once the fonts are ready, its anchor is drawn,
// no load frame is still loading and every animation that ends has ended.
//
// Usage (after `npm --prefix app/web run build`; needs Playwright in tools/poison-compare/js, and
// a Chromium that Playwright can start, or Chrome/Edge installed):
//   node tools/overflow-check.mjs                        exit 1 on any hit
//   node tools/overflow-check.mjs --sizes 1440x900       fewer sizes
//   node tools/overflow-check.mjs --views fans,peripherals
//   node tools/overflow-check.mjs --shots <dir> <tag>    save <dir>/<tag>-<view>-<WxH>-<scheme>.png of every row
//   node tools/overflow-check.mjs --out <dir>            serve another export than app/web/out
//   node tools/overflow-check.mjs --mutate-overflow      put a 40 px wider element in one box per
//                                                        view: every row must then FAIL, and say
//                                                        that the check caught it
//   node tools/overflow-check.mjs --hover-tips           hover every control that draws a tooltip
//                                                        and FAIL when a shown bubble passes the
//                                                        window by more than 1 px, or is cut off
//                                                        by an ancestor that clips it (#271). It
//                                                        replaces the edge measure above for the
//                                                        run; combine with --views and --sizes.

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "..");
const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  return i === -1 ? null : args[i + 1];
};
const out = path.resolve(flag("--out") ?? path.join(repo, "app", "web", "out"));
// Playwright lives in tools/poison-compare/js (as for tools/nav-check.mjs). A worktree that has not
// run `npm ci` there can point PLAYWRIGHT_DIR at another checkout's copy of that folder.
const pwDir = [process.env.PLAYWRIGHT_DIR, path.join(repo, "tools", "poison-compare", "js")]
  .filter(Boolean)
  .find((d) => fs.existsSync(path.join(d, "node_modules", "playwright")));
if (!pwDir) throw new Error("Playwright not found: run `npm ci` in tools/poison-compare/js, or set PLAYWRIGHT_DIR");
const require = createRequire(path.join(pwDir, "package.json"));
const { chromium } = require("playwright");

const shotsDir = flag("--shots");
const shotTag = shotsDir ? args[args.indexOf("--shots") + 2] : null;
const mutate = args.includes("--mutate-overflow");
const hoverTips = args.includes("--hover-tips");
const SIZES = (flag("--sizes") ?? "1920x1040,1440x900,1280x860,1024x768,960x1032,390x844")
  .split(",")
  .map((s) => s.split("x").map(Number));
const SCHEMES = ["dark", "light"];
const SLACK_PX = 1;
const T = 20000;
const PHONE_MAX = 720;

const hasCards = (page) => page.locator(".fan-cards").count().then((n) => n > 0);

/** Fans: pick the CPU fan, then set a "Don't exceed" limit so Reset to automatic is drawn. */
async function fansWithLimit(page) {
  await page.waitForSelector('[aria-label="Select CPU fan"], .fan-cards button', { timeout: T });
  if (await hasCards(page)) {
    await page.locator(".fan-cards button", { hasText: "CPU fan" }).first().click();
  }
  const f = page.getByLabel("Don't exceed temperature", { exact: true });
  await f.fill("70");
  await f.press("Enter");
  // The key is drawn (the hidden copy that holds its room is aria-hidden), and it is no longer
  // sending the limit.
  await page.waitForSelector('button:not([aria-hidden="true"]):has-text("Reset to automatic"):not([aria-busy="true"])', { timeout: T });
}

/**
 * Every view of the demo, and the states of it that draw more than the first paint does.
 * `boxes` adds row containers to measure besides `.panel` and `.rule`.
 */
const VIEWS = [
  { name: "layout", url: "/" },
  { name: "effects", url: "/effects/", boxes: ".effect-toolbar, .effect-card" },
  { name: "devices", url: "/devices/", boxes: ".rd-dev" },
  { name: "displays", url: "/displays/" },
  { name: "peripherals", url: "/peripherals/" },
  { name: "peripherals-read", url: "/peripherals/?state=reads-on" },
  { name: "fans", url: "/fans/" },
  { name: "fans-limit", url: "/fans/", prep: fansWithLimit },
  { name: "settings", url: "/settings/" },
  // No tray in a plain browser: the page waits, then says so (one panel). `?update=available` is
  // the fixture with a release waiting: its update card and the notes' panels.
  { name: "updates", url: "/updates/" },
  { name: "updates-available", url: "/updates/?update=available" },
];
/**
 * The views known to draw no `.panel` (Devices is a list of rows; Effects on a phone is the
 * gallery). They are measured through their `boxes` instead, and the check fails if those are
 * missing too. Any other view without a panel is a failure, not a pass.
 */
const NO_PANEL = (view, width) => view === "devices" || (view === "effects" && width <= PHONE_MAX);

const only = flag("--views")?.split(",");
const rows = VIEWS.filter((v) => !only || only.some((o) => v.name === o || v.name.startsWith(`${o}-`)));

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".txt": "text/plain; charset=utf-8",
  ".ico": "image/x-icon",
};

function resolvePath(urlPath) {
  const clean = decodeURIComponent(urlPath.split("?")[0]);
  let file = path.join(out, clean);
  if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    const asIndex = path.join(out, clean, "index.html");
    if (fs.existsSync(asIndex)) file = asIndex;
  }
  return file.startsWith(out) && fs.existsSync(file) && fs.statSync(file).isFile() ? file : null;
}

const server = http.createServer((req, res) => {
  const file = resolvePath(req.url ?? "/");
  if (!file) {
    res.writeHead(404, { "content-type": "text/plain" });
    res.end("not found");
    return;
  }
  res.writeHead(200, { "content-type": MIME[path.extname(file)] ?? "application/octet-stream" });
  fs.createReadStream(file).pipe(res);
});

/** Runs in the page: everything wrong with the right edges. */
const MEASURE = ({ slack, extraBoxes, mutate }) => {
  const BOX = [".panel", ".rule", extraBoxes].filter(Boolean).join(", ");
  const vw = document.documentElement.clientWidth;
  // A box that is in the page but not drawn (a skeleton kept for its height, `visibility: hidden`)
  // is not a box to measure, nor to put the mutation in.
  const drawnBox = (b) => b.getBoundingClientRect().width > 0 && b.checkVisibility({ visibilityProperty: true, opacityProperty: true });
  // `--mutate-overflow`: a 40 px wider element in the first drawn box, put in and measured in the
  // same call so the view cannot redraw in between. The check has to catch it, or it is not looking.
  let mutated = false;
  if (mutate) {
    const box = [...document.querySelectorAll(BOX)].find(drawnBox);
    if (box) {
      if (getComputedStyle(box).position === "static") box.style.position = "relative";
      const m = document.createElement("div");
      m.className = "ovf-mutant";
      m.style.cssText = "position:absolute;top:0;left:0;height:4px;width:calc(100% + 40px);pointer-events:none";
      box.appendChild(m);
      mutated = true;
    }
  }
  const name = (el) => {
    const cls = String(el.getAttribute("class") ?? "").trim().split(/\s+/).filter(Boolean).slice(0, 2).join(".");
    return `${el.tagName.toLowerCase()}${cls ? `.${cls}` : ""}`;
  };
  const text = (el) => (el.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40);
  const round = (n) => Math.round(n * 10) / 10;
  // A scroll container that scrolls on x is meant to be wider than what it shows. A `.panel` is
  // never one of them: it is the thing being measured.
  const scrollsX = (el) => {
    if (el.matches(".panel")) return false;
    const o = getComputedStyle(el).overflowX;
    return o === "auto" || o === "scroll";
  };
  // Not drawn, or drawn for a screen reader only: not an overflow. A tooltip waits at opacity 0
  // until the pointer is over its mark.
  const notDrawn = (el) => {
    const cs = getComputedStyle(el);
    return cs.display === "none" || cs.visibility === "hidden" || cs.opacity === "0";
  };
  /**
   * Is `el` exempt: not drawn, fixed (when asked), inside a scroll-x container below `stop`, or
   * (`clipAt`, for the window check) cut off by an ancestor that clips on x at or inside that edge,
   * so the part that passes it is never drawn.
   */
  const skipped = (el, stop, { fixed, clipAt }) => {
    for (let a = el; a && a !== stop; a = a.parentElement) {
      if (notDrawn(a)) return true;
      if (fixed && getComputedStyle(a).position === "fixed") return true;
      if (a !== el && scrollsX(a)) return true;
      if (clipAt !== undefined && a !== el && !a.matches(".panel")) {
        const ox = getComputedStyle(a).overflowX;
        if (ox !== "visible" && a.getBoundingClientRect().right <= clipAt) return true;
      }
    }
    return false;
  };
  const tooSmall = (r) => r.width <= 2 || r.height <= 2;
  const hits = [];

  // 1. Elements past the right edge of their box, and boxes past the box that holds them.
  const boxes = [...document.querySelectorAll(BOX)].filter(drawnBox);
  const inBox = new Set();
  for (const box of boxes) {
    const br = box.getBoundingClientRect();
    for (const el of box.querySelectorAll("*")) {
      if (el.closest(BOX) !== box) continue;
      const r = el.getBoundingClientRect();
      if (tooSmall(r) || r.right <= br.right + slack) continue;
      if (skipped(el, box, { fixed: true })) continue;
      inBox.add(el);
      el.__box = box;
    }
    // The box itself against the box above it (a `.rule` against its panel).
    const parent = box.parentElement?.closest(BOX);
    if (parent && box.getBoundingClientRect().right > parent.getBoundingClientRect().right + slack && !skipped(box, parent, { fixed: true })) {
      inBox.add(box);
      box.__box = parent;
    }
    // A box that scrolls sideways is a box whose content is wider than it.
    const ox = getComputedStyle(box).overflowX;
    if ((ox === "auto" || ox === "scroll") && box.scrollWidth > box.clientWidth + slack) {
      hits.push({ kind: "scrolls", el: name(box), text: "", over: box.scrollWidth - box.clientWidth });
    }
  }
  for (const el of [...inBox].filter((e) => !(e.parentElement && inBox.has(e.parentElement)))) {
    const over = el.getBoundingClientRect().right - el.__box.getBoundingClientRect().right;
    hits.push({ kind: "box", el: name(el), box: name(el.__box), text: text(el), over: round(over) });
  }

  // 2. Anything past the window, in a panel or not, and panels wider than the window.
  const past = new Set();
  for (const el of document.querySelectorAll("body *")) {
    const r = el.getBoundingClientRect();
    if (tooSmall(r) || r.right <= vw + slack) continue;
    if (skipped(el, document.body, { fixed: false, clipAt: vw + slack })) continue;
    past.add(el);
  }
  for (const el of [...past].filter((e) => !(e.parentElement && past.has(e.parentElement)))) {
    hits.push({ kind: "window", el: name(el), text: text(el), over: round(el.getBoundingClientRect().right - vw) });
  }
  for (const p of document.querySelectorAll(".panel")) {
    const w = p.getBoundingClientRect().width;
    if (w > vw + slack) hits.push({ kind: "wide", el: name(p), text: "", over: round(w - vw) });
  }
  const sw = document.documentElement.scrollWidth;
  if (sw > vw + slack) hits.push({ kind: "doc", el: "html", text: "", over: sw - vw });

  // 3. A unit that has wrapped under its figure: its middle is below the figure's bottom.
  for (const input of document.querySelectorAll("input.num")) {
    const unit = input.parentElement?.querySelector(".unit");
    if (!unit || unit.getBoundingClientRect().width === 0 || skipped(input, document.body, { fixed: true })) continue;
    const ir = input.getBoundingClientRect();
    const ur = unit.getBoundingClientRect();
    if (ur.top + ur.height / 2 > ir.bottom) {
      hits.push({ kind: "wrapped", el: name(unit), text: text(unit), over: round(ur.top + ur.height / 2 - ir.bottom) });
    }
  }
  return { vw, mutated, boxes: boxes.length, panels: boxes.filter((b) => b.matches(".panel")).length, hits };
};

const describe = (x) => {
  switch (x.kind) {
    case "box":
      return `${x.el} "${x.text}" passes ${x.box} by ${x.over}px`;
    case "window":
      return `${x.el} "${x.text}" passes the window's right edge by ${x.over}px`;
    case "window-left":
      return `${x.el} "${x.text}" passes the window's left edge by ${x.over}px`;
    case "window-top":
      return `${x.el} "${x.text}" passes the window's top edge by ${x.over}px`;
    case "window-bottom":
      return `${x.el} "${x.text}" passes the window's bottom edge by ${x.over}px`;
    case "clipped":
      return `${x.el} "${x.text}" is cut off by ${x.clipper} at its ${x.edge} by ${x.over}px`;
    case "wide":
      return `${x.el} is ${x.over}px wider than the window`;
    case "doc":
      return `the document scrolls sideways by ${x.over}px`;
    case "scrolls":
      return `${x.el} scrolls sideways: its content is ${x.over}px wider than it`;
    case "wrapped":
      return `${x.el} "${x.text}" has wrapped under its figure, ${x.over}px below the figure's bottom`;
    default:
      return JSON.stringify(x);
  }
};

/** Runs in the page: every drawn control that has a tooltip, in DOM order. */
const TIPS = () => {
  const drawn = (el) => {
    const cs = getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden") return false;
    const r = el.getBoundingClientRect();
    return r.width > 1 && r.height > 1;
  };
  return [...document.querySelectorAll(".tip")]
    .map((tip, index) => {
      if (!drawn(tip)) return null;
      const b = tip.querySelector(".tip-bubble");
      // A bubble its own CSS never draws (display: none) has no placement to check.
      if (!b || !drawn(b)) return null;
      return { index, text: (b.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40) };
    })
    .filter(Boolean);
};

/** Runs in the page after the hover: the shown bubble against the window and its clippers. */
const MEASURE_TIP = ({ index, slack }) => {
  const tip = document.querySelectorAll(".tip")[index];
  const b = tip?.querySelector(".tip-bubble");
  if (!b) return { missing: true };
  const name = (el) => {
    const cls = String(el.getAttribute("class") ?? "").trim().split(/\s+/).filter(Boolean).slice(0, 2).join(".");
    return `${el.tagName.toLowerCase()}${cls ? `.${cls}` : ""}`;
  };
  const round = (n) => Math.round(n * 10) / 10;
  const r = b.getBoundingClientRect();
  const vw = document.documentElement.clientWidth;
  const vh = document.documentElement.clientHeight;
  const text = (b.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40);
  const hits = [];
  if (r.right > vw + slack) hits.push({ kind: "window", el: name(b), text, over: round(r.right - vw) });
  if (r.left < -slack) hits.push({ kind: "window-left", el: name(b), text, over: round(-r.left) });
  if (r.bottom > vh + slack) hits.push({ kind: "window-bottom", el: name(b), text, over: round(r.bottom - vh) });
  if (r.top < -slack) hits.push({ kind: "window-top", el: name(b), text, over: round(-r.top) });
  // An ancestor that clips the bubble cuts its words off even where the window has room.
  for (let a = b.parentElement; a && a !== document.body; a = a.parentElement) {
    const cs = getComputedStyle(a);
    if (cs.overflowX === "visible" && cs.overflowY === "visible") continue;
    const ar = a.getBoundingClientRect();
    const cuts = [];
    if (cs.overflowX !== "visible") {
      if (r.right > ar.right + slack) cuts.push(["right", r.right - ar.right]);
      if (r.left < ar.left - slack) cuts.push(["left", ar.left - r.left]);
    }
    if (cs.overflowY !== "visible") {
      if (r.bottom > ar.bottom + slack) cuts.push(["bottom", r.bottom - ar.bottom]);
      if (r.top < ar.top - slack) cuts.push(["top", ar.top - r.top]);
    }
    if (cuts.length) {
      const worst = cuts.sort((x, y) => y[1] - x[1])[0];
      hits.push({ kind: "clipped", el: name(b), clipper: name(a), edge: worst[0], over: round(worst[1]), text });
      break;
    }
  }
  return { opacity: Number(getComputedStyle(b).opacity), hits, text };
};

/** Hovers every tooltip the page draws, and measures each bubble once it is shown. */
async function hoverEveryTip(page) {
  const tips = await page.evaluate(TIPS);
  const hits = [];
  let hovered = 0;
  for (const tip of tips) {
    try {
      await page.locator(".tip").nth(tip.index).hover({ timeout: 10000 });
    } catch (e) {
      hits.push(`"${tip.text}": could not hover it (${String(e.message).split("\n")[0]})`);
      continue;
    }
    try {
      await page.waitForFunction(
        (i) => {
          const b = document.querySelectorAll(".tip")[i]?.querySelector(".tip-bubble");
          return b ? Number(getComputedStyle(b).opacity) > 0.99 : false;
        },
        tip.index,
        { timeout: 4000 },
      );
    } catch {
      // Measured below anyway: a bubble that never showed is reported as its own failure.
    }
    const m = await page.evaluate(MEASURE_TIP, { index: tip.index, slack: SLACK_PX });
    if (m.missing) {
      hits.push(`"${tip.text}": its bubble left the page while it was hovered`);
      continue;
    }
    if (m.opacity < 0.99) hits.push(`"${m.text}": hovered but the bubble did not show (opacity ${m.opacity})`);
    else hovered++;
    for (const h of m.hits) hits.push(describe(h));
  }
  return { tips: tips.length, hovered, hits };
}

/** A busy machine can time a request out: up to three tries, from a blank page. */
async function goto(page, url) {
  for (let attempt = 1; ; attempt++) {
    try {
      await page.goto(url, { waitUntil: "load", timeout: 30000 });
      return;
    } catch (e) {
      if (attempt === 3) throw e;
      await page.goto("about:blank").catch(() => {});
      await page.waitForTimeout(1000);
    }
  }
}

/**
 * Wait until the view is drawn and still: fonts ready, the anchor on screen, no load frame
 * waiting, every animation that ends has ended (the ones that loop, a spinner, may run), and two
 * frames since.
 */
async function settle(page, anchor) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForFunction(
    (sel) => {
      // Any one of them drawn: the first match can be a sheet that is shut on a phone.
      const drawn = [...document.querySelectorAll(sel)].some((el) => {
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      });
      if (!drawn) return false;
      if (document.querySelector('[data-state="loading"], [aria-busy="true"]')) return false;
      return document.getAnimations().every((a) => a.playState === "finished" || a.effect?.getTiming().iterations === Infinity);
    },
    anchor,
    { timeout: T },
  );
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
}

async function main() {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  let browser;
  let channel = "chromium";
  try {
    browser = await chromium.launch({ headless: true });
  } catch {
    // No downloaded Chromium (a CI runner): the Chrome that comes with the runner will do.
    channel = "chrome";
    browser = await chromium.launch({ headless: true, channel: "chrome" });
  }
  console.log(`overflow-check: browser channel ${channel}, ${rows.length} views x ${SIZES.length} sizes x ${SCHEMES.length} schemes`);
  if (hoverTips) console.log("overflow-check: HOVER mode: every drawn tooltip is hovered and its shown bubble measured");
  if (mutate) console.log("overflow-check: MUTATION on, one box per view gets a 40px wider element: every row must FAIL");
  const failures = [];
  let checks = 0;
  let caught = 0;
  let missed = 0;
  let tipTotal = 0;
  let hoverTotal = 0;
  try {
    for (const [w, h] of SIZES) {
      for (const scheme of SCHEMES) {
        const ctx = await browser.newContext({
          viewport: { width: w, height: h },
          colorScheme: scheme,
          isMobile: w <= PHONE_MAX,
          hasTouch: w <= PHONE_MAX,
        });
        await ctx.addInitScript((s) => {
          try {
            localStorage.setItem("canvas-rgb.theme", s);
            localStorage.setItem("canvas-rgb.demo-onboarded", "1");
            // "Later" in this launch: the first-run onboarding would take the whole window.
            sessionStorage.setItem("canvas-rgb.launch", "overflow-check");
            localStorage.setItem("canvas-rgb.onboarding-later", "overflow-check");
          } catch {
            /* storage blocked */
          }
        }, scheme);
        const page = await ctx.newPage();
        for (const v of rows) {
          const label = `${w}x${h} ${scheme} ${v.name}`;
          const known = NO_PANEL(v.name.split("-")[0], w);
          const extraBoxes = v.boxes ?? "";
          // A known no-panel view is anchored on its own rows; any other on a panel.
          const anchor = known ? extraBoxes : ".panel";
          const bad = [];
          let m = null;
          let detail = "-";
          try {
            await goto(page, `${base}${v.url}`);
            await settle(page, anchor);
            if (v.prep) {
              await v.prep(page);
              await settle(page, anchor);
            }
            if (hoverTips) {
              const h = await hoverEveryTip(page);
              tipTotal += h.tips;
              hoverTotal += h.hovered;
              detail = `${h.tips} tips, ${h.hovered} hovered`;
              for (const b of h.hits) bad.push(b);
              if (h.tips === 0) bad.push("nothing was measured: the view draws no tooltip");
            } else {
              m = await page.evaluate(MEASURE, { slack: SLACK_PX, extraBoxes, mutate });
              detail = m ? `${m.panels} panels, ${m.boxes - m.panels} other boxes` : "-";
              if (mutate && !m.mutated) bad.push("mutation: no drawn box to put the 40px element in");
              for (const x of m.hits) bad.push(describe(x));
              if (m.boxes === 0) bad.push("nothing was measured: no box drawn");
              else if (m.panels === 0 && !known) bad.push("nothing was measured: no .panel drawn, and the view is not on the NO_PANEL list");
            }
            if (shotsDir) {
              fs.mkdirSync(shotsDir, { recursive: true });
              await page.screenshot({ path: path.join(shotsDir, `${shotTag}-${v.name}-${w}x${h}-${scheme}.png`) });
            }
          } catch (e) {
            bad.push(`could not measure: ${String(e.message).split("\n")[0]}`);
          }
          if (mutate && !hoverTips) {
            const ok = bad.some((b) => b.includes("ovf-mutant"));
            if (ok) caught++;
            else {
              missed++;
              bad.push("MUTATION MISSED: the 40px element was not reported");
            }
          }
          checks++;
          console.log(`${bad.length ? "FAIL" : "ok  "} ${label}  [${detail}]`);
          for (const b of bad) {
            console.log(`       - ${b}`);
            failures.push(`${label}: ${b}`);
          }
        }
        await ctx.close();
      }
    }
  } finally {
    await browser.close();
    server.close();
  }
  console.log(`\n${checks} view checks, ${failures.length} hits`);
  if (hoverTips) console.log(`${tipTotal} tooltips hovered (${hoverTotal} showed their bubble) over ${checks} rows`);
  if (mutate) console.log(`mutation caught in ${caught} of ${checks} rows, missed in ${missed}`);
  if (checks === 0 || failures.length) process.exitCode = 1;
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
