// The left rail must never scroll sideways, measured in a real browser (issue #467, guard #478).
//
// jsdom has no layout, so a unit test cannot see a rail whose scrollWidth exceeds its clientWidth.
// This serves the static export (app/web/out) on loopback, opens each size below in both colour
// schemes, and measures `nav.rail`:
//   - scrollWidth - clientWidth: the sideways scroll the user sees (#467);
//   - computed overflow-x/overflow-y: `overflow-y: auto` alone makes overflow-x *compute* to
//     auto, which is why a tooltip that is not fixed used to give the rail a sideways bar;
//   - the rail-link ::after labels and the rail's .tip-bubbles: read through getComputedStyle,
//     because a real element cannot see a pseudo-element. A fixed bubble is allowed to reach over
//     the page; it must simply stay out of the rail's scroll box.
// It also checks that a rail tall enough to scroll still scrolls vertically, so a fix cannot pass
// by making the rail unscrollable.
//
// Usage (after `npm --prefix app/web run build`; needs Playwright in tools/poison-compare/js, or
// PLAYWRIGHT_DIR pointing at another checkout's copy, and a Chromium or Chrome/Edge to start):
//   node tools/rail-check.mjs                          exit 1 on any sideways scroll
//   node tools/rail-check.mjs --out <dir>              serve another export (before/after builds)
//   node tools/rail-check.mjs --sizes 1440x900,1024x768 --views-only
//   node tools/rail-check.mjs --sweep                  sweep 670-830 and 1230-1330 px, 1 px steps
//   node tools/rail-check.mjs --shots <dir> <tag>      also save <dir>/<tag>-<WxH>-<scheme>-hover.png
//                                                     of a hovered rail tooltip
// Print the version and git SHA in the tag so each shot names its own build.

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
const shotsDir = flag("--shots");
const shotTag = shotsDir ? args[args.indexOf("--shots") + 2] : null;
const SIZES = (flag("--sizes") ?? "1920x1040,1440x900,1280x800,1024x768,768x1024,390x844,1440x600,1280x500,1440x400")
  .split(",")
  .map((s) => s.split("x").map(Number));
const SCHEMES = ["dark", "light"];
const doSweep = args.includes("--sweep");
// The slack a scrollbar or a subpixel rounding can cost without being a real sideways scroll.
export const SLACK = 1;

// Playwright is loaded inside main(), so judge() can be imported by tools/tests without needing
// the browser install. The CLI behaviour is unchanged.
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
  const abs = path.resolve(file);
  return abs.startsWith(out + path.sep) && fs.existsSync(abs) && fs.statSync(abs).isFile() ? abs : null;
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

/** Runs in the page: the rail's scroll box and what is placed around it. */
const MEASURE = () => {
  const rail = document.querySelector("nav.rail");
  if (!rail) return { error: "no nav.rail" };
  const rr = rail.getBoundingClientRect();
  const cs = getComputedStyle(rail);
  const nameOf = (el) => {
    const cls = el.className && el.className.baseVal !== undefined ? el.className.baseVal : el.className;
    return `${el.tagName.toLowerCase()}${cls ? "." + String(cls).trim().split(/\s+/).slice(0, 2).join(".") : ""}`;
  };
  const bubbles = [...rail.querySelectorAll(".tip-bubble")]
    .filter((b) => getComputedStyle(b).position === "fixed")
    .map((b) => ({ text: (b.textContent ?? "").trim().slice(0, 50) }));
  const labels = [...rail.querySelectorAll("a.rail-link")]
    .map((a) => {
      const s = getComputedStyle(a, "::after");
      return { label: a.getAttribute("data-label"), left: s.left, width: s.width, opacity: s.opacity };
    })
    .filter((x) => x.width && x.width !== "auto");
  return {
    vw: document.documentElement.clientWidth,
    vh: window.innerHeight,
    scrollWidth: rail.scrollWidth,
    clientWidth: rail.clientWidth,
    overflowX: rail.scrollWidth - rail.clientWidth,
    scrollHeight: rail.scrollHeight,
    clientHeight: rail.clientHeight,
    overflowY: rail.scrollHeight - rail.clientHeight,
    computedOverflowX: cs.overflowX,
    computedOverflowY: cs.overflowY,
    rail: { left: +rr.left.toFixed(1), right: +rr.right.toFixed(1), width: +rr.width.toFixed(1) },
    fixedBubbles: bubbles.length,
    labels: labels.length,
  };
};

/**
 * The user-visible verdict: a rail that scrolls sideways. The computed overflow-x being `auto`
 * is not itself a failure (a vertical rail is written `overflow-y: auto` and computes `auto` on
 * x too); only the measured scrollWidth beating clientWidth is.
 */
export function judge(m) {
  const bad = [];
  if (m.error) return [m.error];
  if (m.overflowX > SLACK) {
    bad.push(
      `the rail scrolls sideways by ${m.overflowX}px (scrollWidth ${m.scrollWidth} > clientWidth ${m.clientWidth}; overflow-x computes to ${m.computedOverflowX})`,
    );
  }
  return bad;
}

async function newPage(browser, w, h, scheme) {
  const ctx = await browser.newContext({
    viewport: { width: w, height: h },
    colorScheme: scheme,
    isMobile: w <= 720,
    hasTouch: w <= 720,
  });
  await ctx.addInitScript((s) => {
    try {
      localStorage.setItem("canvas-rgb.theme", s);
      // "Later" in this launch: the first-run onboarding would cover the whole window.
      sessionStorage.setItem("canvas-rgb.launch", "rail-check");
      localStorage.setItem("canvas-rgb.onboarding-later", "rail-check");
    } catch {
      /* storage blocked */
    }
  }, scheme);
  const page = await ctx.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/`, { waitUntil: "load" });
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(400);
  return { ctx, page };
}

const fmt = (m) =>
  m.error
    ? m.error
    : `scrollX=${m.overflowX}px (sw=${m.scrollWidth} cw=${m.clientWidth}) scrollY=${m.overflowY}px overflow=${m.computedOverflowX}/${m.computedOverflowY} railW=${m.rail.width} fixedBubbles=${m.fixedBubbles} labels=${m.labels}`;

async function hover(page, w) {
  const link = page.locator("nav.rail a.rail-link").first();
  if (await link.count()) {
    await link.hover();
  } else if (w > 1279) {
    const tip = page.locator("nav.rail .tip").first();
    if (await tip.count()) await tip.hover();
  }
  await page.waitForTimeout(250);
}

async function main() {
  const pwDir = [
    process.env.PLAYWRIGHT_DIR && path.resolve(process.env.PLAYWRIGHT_DIR),
    path.join(repo, "tools", "poison-compare", "js"),
  ]
    .filter(Boolean)
    .find((d) => fs.existsSync(path.join(d, "node_modules", "playwright")));
  if (!pwDir) throw new Error("Playwright not found: run `npm ci` in tools/poison-compare/js, or set PLAYWRIGHT_DIR");
  const require = createRequire(path.join(pwDir, "package.json"));
  const { chromium } = require("playwright");
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  let browser;
  let channel = "chromium";
  try {
    browser = await chromium.launch({ headless: true });
  } catch {
    channel = "chrome";
    browser = await chromium.launch({ headless: true, channel: "chrome" });
  }
  console.log(`rail-check: tag=${shotTag ?? "untagged"} browser=${channel} serve=${out}`);
  const failures = [];
  let checks = 0;
  try {
    for (const [w, h] of SIZES) {
      for (const scheme of SCHEMES) {
        const { ctx, page } = await newPage(browser, w, h, scheme);
        const base = await page.evaluate(MEASURE);
        checks++;
        console.log(`${w}x${h} ${scheme} base      ${fmt(base)}`);
        for (const b of judge(base)) failures.push(`${w}x${h} ${scheme} base: ${b}`);
        // Vertical scroll must still work: a fix cannot pass by freezing the rail.
        const vscroll = await page.evaluate(() => {
          const rail = document.querySelector("nav.rail");
          const before = rail.scrollTop;
          rail.scrollTop = 10000;
          const reached = rail.scrollTop;
          rail.scrollTop = before;
          return { max: rail.scrollHeight - rail.clientHeight, reached };
        });
        if (vscroll.max > SLACK && vscroll.reached <= 0) {
          failures.push(`${w}x${h} ${scheme}: vertical scroll did not move (max=${vscroll.max})`);
        }
        await hover(page, w);
        const hov = await page.evaluate(MEASURE);
        checks++;
        console.log(`${w}x${h} ${scheme} hovered  ${fmt(hov)}`);
        for (const b of judge(hov)) failures.push(`${w}x${h} ${scheme} hover: ${b}`);
        if (shotsDir && (w === 1440 || w === 1024) && h <= 900) {
          fs.mkdirSync(shotsDir, { recursive: true });
          await page.screenshot({ path: path.join(shotsDir, `${shotTag}-${w}x${h}-${scheme}-hover.png`) });
        }
        await ctx.close();
      }
    }

    if (doSweep) {
      for (const [lo, hi] of [
        [670, 830],
        [1230, 1330],
      ]) {
        let maxX = 0;
        let maxAt = null;
        const { ctx, page } = await newPage(browser, lo, 900, "dark");
        for (let w = lo; w <= hi; w++) {
          await page.setViewportSize({ width: w, height: 900 });
          await page.waitForTimeout(35);
          const m = await page.evaluate(MEASURE);
          if (m.overflowX > maxX) {
            maxX = m.overflowX;
            maxAt = w;
          }
        }
        await ctx.close();
        console.log(`sweep ${lo}-${hi}px @900h: max sideways ${maxX}px at ${maxAt}px`);
        if (maxX > SLACK) failures.push(`sweep ${lo}-${hi}: max sideways ${maxX}px at ${maxAt}px`);
      }
    }
  } finally {
    await browser.close();
    server.close();
  }
  console.log(`\n${checks} measurements, ${failures.length} sideways-scroll failures`);
  for (const f of failures) console.log("  FAIL " + f);
  if (failures.length) process.exitCode = 1;
}

// Only run the CLI when invoked as a script; tools/tests imports judge() without a browser.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((e) => {
    console.error(e);
    process.exit(2);
  });
}
