// Controls that run past their panel, measured in a real browser (issues #263, #268).
//
// jsdom has no layout, so a unit test cannot see a select that is wider than the Lighting panel
// or a "Reset to automatic" key that sticks out of its note. This serves the static export
// (app/web/out) on loopback, opens every view of the demo at the six redesign sizes in both colour
// schemes, and reports every element inside a `.panel` (or inside a `.rule`, the boxed note
// some panels hold) whose right edge passes that box's right edge by more than 1 px. An element inside a scroll container that scrolls on x (the container
// is meant to be wider than what it shows) is left out, and so is an element that is not drawn
// (display none, hidden, opacity 0: a tooltip before hover), is only there for a screen reader
// (1-2 px) or is fixed to the window (a toast, a dialog).
//
// A figure's unit that has wrapped under the figure ("70" over "°C", #268) is reported too: it
// stays inside the box but is no longer a row.
//
// Only the topmost offender in a chain is reported: when a row overflows, its children do too,
// and listing each would bury the row.
//
// Some states only exist after a click: the Fans "Don't exceed" row shows "Reset to automatic"
// once a limit is set, and the Peripherals page shows the Attack Shark X6's Lighting panel once
// the device has been read. Those are extra rows in VIEWS below, with a `prep` that gets there.
//
// Usage (after `npm --prefix app/web run build`; needs Playwright in tools/poison-compare/js, and
// a Chromium that Playwright can start, or Chrome/Edge installed):
//   node tools/overflow-check.mjs                        exit 1 on any hit
//   node tools/overflow-check.mjs --sizes 1440x900       fewer sizes
//   node tools/overflow-check.mjs --views fans,peripherals
//   node tools/overflow-check.mjs --shots <dir> <tag>    save <dir>/<tag>-<view>-<WxH>-<scheme>.png of every row
//   node tools/overflow-check.mjs --out <dir>            serve another export than app/web/out

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
const SIZES = (flag("--sizes") ?? "1920x1040,1440x900,1280x860,1024x768,960x1032,390x844")
  .split(",")
  .map((s) => s.split("x").map(Number));
const SCHEMES = ["dark", "light"];
const SLACK_PX = 1;
const T = 20000;

const hasCards = (page) => page.locator(".fan-cards").count().then((n) => n > 0);

/** Fans: pick the CPU fan, then set a "Don't exceed" limit so Reset to automatic is drawn. */
async function fansWithLimit(page) {
  await page.waitForSelector('[aria-label="Select CPU fan"], .fan-cards button', { timeout: T });
  if (await hasCards(page)) {
    await page.locator(".fan-cards button", { hasText: "CPU fan" }).first().click();
    await page.waitForTimeout(300);
  }
  const f = page.getByLabel("Don't exceed temperature", { exact: true });
  await f.fill("70");
  await f.press("Enter");
  await page.waitForSelector('button:has-text("Reset to automatic")', { timeout: T });
  await page.waitForTimeout(600);
}

/** Every view of the demo, and the states of it that draw more than the first paint does. */
const VIEWS = [
  { name: "layout", url: "/" },
  { name: "effects", url: "/effects/" },
  { name: "devices", url: "/devices/" },
  { name: "displays", url: "/displays/" },
  { name: "peripherals", url: "/peripherals/" },
  { name: "peripherals-read", url: "/peripherals/?state=reads-on" },
  { name: "fans", url: "/fans/" },
  { name: "fans-limit", url: "/fans/", prep: fansWithLimit },
  { name: "settings", url: "/settings/" },
  { name: "updates", url: "/updates/" },
];
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

/** Runs in the page: every element that passes the right edge of the `.panel` or `.rule` it sits in. */
const MEASURE = (slack) => {
  const name = (el) => {
    const cls = String(el.getAttribute("class") ?? "").trim().split(/\s+/).filter(Boolean).slice(0, 2).join(".");
    return `${el.tagName.toLowerCase()}${cls ? `.${cls}` : ""}`;
  };
  // A scroll container that scrolls on x is meant to be wider than what it shows.
  const scrollsX = (el) => {
    const o = getComputedStyle(el).overflowX;
    return o === "auto" || o === "scroll";
  };
  const skipped = (el, panel) => {
    for (let a = el; a; a = a.parentElement) {
      const cs = getComputedStyle(a);
      if (cs.position === "fixed" || cs.display === "none" || cs.visibility === "hidden") return true;
      // A tooltip waits at opacity 0 until the pointer is over its mark: it is not drawn yet.
      if (cs.opacity === "0") return true;
      if (a !== el && scrollsX(a)) return true;
      if (a === panel) return false;
    }
    return false;
  };
  const hits = new Set();
  const BOX = ".panel, .rule";
  const panels = [...document.querySelectorAll(BOX)].filter((p) => p.getBoundingClientRect().width > 0);
  for (const panel of panels) {
    const pr = panel.getBoundingClientRect();
    for (const el of panel.querySelectorAll("*")) {
      if (el.closest(BOX) !== panel) continue;
      const r = el.getBoundingClientRect();
      if (r.width <= 2 || r.height <= 2) continue;
      if (r.right <= pr.right + slack) continue;
      if (skipped(el, panel)) continue;
      el.__panel = panel;
      hits.add(el);
    }
  }
  const top = [...hits].filter((el) => !(el.parentElement && hits.has(el.parentElement)));
  // A unit that has wrapped under its figure: its middle is below the figure's bottom.
  const wrapped = [];
  for (const input of document.querySelectorAll("input.num")) {
    const unit = input.parentElement?.querySelector(".unit");
    if (!unit || unit.getBoundingClientRect().width === 0 || skipped(input, input.closest(BOX) ?? document.body)) continue;
    const ir = input.getBoundingClientRect();
    const ur = unit.getBoundingClientRect();
    if (ur.top + ur.height / 2 > ir.bottom) {
      wrapped.push({
        el: name(unit),
        panel: name(input.closest(BOX) ?? document.body),
        text: (unit.textContent ?? "").trim(),
        over: Math.round((ur.top + ur.height / 2 - ir.bottom) * 10) / 10,
        right: Math.round(ur.right),
        panelRight: Math.round(ir.right),
        wrapped: true,
      });
    }
  }
  return {
    vw: document.documentElement.clientWidth,
    panels: panels.length,
    hits: wrapped.concat(top.map((el) => {
      const r = el.getBoundingClientRect();
      const pr = el.__panel.getBoundingClientRect();
      return {
        el: name(el),
        panel: name(el.__panel),
        text: (el.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40),
        over: Math.round((r.right - pr.right) * 10) / 10,
        right: Math.round(r.right),
        panelRight: Math.round(pr.right),
      };
    })),
  };
};

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
  const failures = [];
  let checks = 0;
  let none = 0; // views drawn with no .panel in them: nothing measured, which is not a pass
  try {
    for (const [w, h] of SIZES) {
      for (const scheme of SCHEMES) {
        const ctx = await browser.newContext({
          viewport: { width: w, height: h },
          colorScheme: scheme,
          isMobile: w <= 720,
          hasTouch: w <= 720,
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
          let bad = [];
          let m = null;
          try {
            await goto(page, `${base}${v.url}`);
            await page.evaluate(() => document.fonts.ready);
            await page.waitForTimeout(900);
            if (v.prep) await v.prep(page);
            m = await page.evaluate(MEASURE, SLACK_PX);
            if (m.panels === 0) none++;
            for (const x of m.hits) {
              bad.push(
                x.wrapped
                  ? `${x.el} "${x.text}" has wrapped under its figure, ${x.over}px below the figure's bottom`
                  : `${x.el} "${x.text}" passes ${x.panel} by ${x.over}px (right ${x.right}, panel right ${x.panelRight})`,
              );
            }
            if (shotsDir) {
              fs.mkdirSync(shotsDir, { recursive: true });
              await page.screenshot({ path: path.join(shotsDir, `${shotTag}-${v.name}-${w}x${h}-${scheme}.png`) });
            }
          } catch (e) {
            bad.push(`could not measure: ${String(e.message).split("\n")[0]}`);
          }
          checks++;
          const status = bad.length ? "FAIL" : m && m.panels === 0 ? "none" : "ok  ";
          console.log(`${status} ${label}  [${m ? `${m.panels} panels` : "-"}]`);
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
  console.log(`\n${checks} view checks, ${failures.length} hits, ${none} drew no .panel (nothing measured: UNKNOWN, not a pass)`);
  if (checks === 0 || failures.length) process.exitCode = 1;
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
