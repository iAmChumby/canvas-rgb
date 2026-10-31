// The phone's bottom nav, measured in a real browser (issue #42).
//
// jsdom has no layout, so the unit tests cannot see a label cut to "Se". This serves the static
// export (app/web/out) on loopback, opens every page at 360, 390 and 430 px in both colour
// schemes, and checks the bottom nav:
//   - all seven pages are in it, and each link sits inside the window;
//   - the label a reader can see is drawn whole (scrollWidth <= clientWidth) and is 14 px or more;
//   - the links do not overlap;
//   - the last thing on the page can be scrolled clear of the nav.
// A label that is hidden on purpose (the quiet links show their icon only) is kept as text for
// screen readers and is not asked to be visible; the check says so by its size, 1 px.
//
// Usage (after `npm --prefix app/web run build`; needs `npm ci` in tools/poison-compare/js for
// Playwright, and a Chromium that Playwright can start, or Chrome/Edge installed):
//   node tools/nav-check.mjs                       exit 1 on any failure
//   node tools/nav-check.mjs --shots <dir> <tag>   also save <dir>/<tag>-<page>-<width>-<scheme>.png
//   node tools/nav-check.mjs --widths 360,390      other widths (1440 checks that the nav is not the bottom bar)

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "..");
const out = path.join(repo, "app", "web", "out");
const require = createRequire(path.join(repo, "tools", "poison-compare", "js", "package.json"));
const { chromium } = require("playwright");

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  return i === -1 ? null : args[i + 1];
};
const shotsDir = flag("--shots");
const shotTag = shotsDir ? args[args.indexOf("--shots") + 2] : null;
const WIDTHS = (flag("--widths") ?? "360,390,430").split(",").map(Number);
const SCHEMES = ["dark", "light"];
const PAGES = [
  { name: "layout", url: "/", label: "Layout" },
  { name: "effects", url: "/effects/", label: "Effects" },
  { name: "devices", url: "/devices/", label: "Devices" },
  { name: "displays", url: "/displays/", label: "Displays" },
  { name: "peripherals", url: "/peripherals/", label: "Peripherals" },
  { name: "fans", url: "/fans/", label: "Fans" },
  { name: "settings", url: "/settings/", label: "Settings" },
];
const MIN_FONT_PX = 14;

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

/** Runs in the page: what the bottom nav looks like now. */
const MEASURE = () => {
  const vw = document.documentElement.clientWidth;
  const nav = document.querySelector("nav.rail");
  if (!nav) return { error: "no nav.rail" };
  const nr = nav.getBoundingClientRect();
  const links = [...nav.querySelectorAll("a.rail-link")].map((a) => {
    const r = a.getBoundingClientRect();
    const label = a.querySelector(".rail-label");
    const lr = label.getBoundingClientRect();
    return {
      text: (label.textContent ?? "").trim(),
      active: a.getAttribute("aria-current") === "page",
      left: r.left,
      right: r.right,
      top: r.top,
      bottom: r.bottom,
      // 1 px wide: the label is kept for screen readers and not drawn.
      hidden: lr.width <= 2 || lr.height <= 2,
      labelScroll: label.scrollWidth,
      labelClient: label.clientWidth,
      labelLeft: lr.left,
      labelRight: lr.right,
      fontPx: parseFloat(getComputedStyle(label).fontSize),
      display: getComputedStyle(a).display,
    };
  });
  // The last thing on the page: scroll to the end, then find the lowest drawn element that is not
  // part of the nav, a toast or a fixed overlay.
  window.scrollTo(0, document.documentElement.scrollHeight);
  let lowest = 0;
  let lowestName = "";
  for (const el of document.querySelectorAll("main *, .content *")) {
    if (el.closest("nav.rail")) continue;
    const cs = getComputedStyle(el);
    if (cs.position === "fixed" || cs.display === "none" || cs.visibility === "hidden") continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    if (r.bottom > lowest) {
      lowest = r.bottom;
      lowestName = `${el.tagName.toLowerCase()}.${String(el.className).split(" ")[0]}`;
    }
  }
  return {
    vw,
    docOverflow: document.documentElement.scrollWidth - vw,
    nav: { top: nr.top, bottom: nr.bottom, left: nr.left, right: nr.right, position: getComputedStyle(nav).position },
    links,
    lowest,
    lowestName,
    vh: window.innerHeight,
  };
};

function judge(m, page, width) {
  const bad = [];
  if (m.error) return [m.error];
  if (m.nav.position !== "fixed") return [`nav is ${m.nav.position}, not the fixed bottom bar`];
  if (m.links.length !== PAGES.length) bad.push(`${m.links.length} links, expected ${PAGES.length}`);
  for (const p of PAGES) {
    if (!m.links.some((l) => l.text === p.label)) bad.push(`no link named "${p.label}"`);
  }
  if (m.docOverflow > 0) bad.push(`page scrolls sideways by ${m.docOverflow}px`);
  let drawn = 0;
  for (const l of m.links) {
    if (l.left < -0.5 || l.right > m.vw + 0.5) bad.push(`${l.text}: link ${l.left.toFixed(0)}..${l.right.toFixed(0)} is outside 0..${m.vw}`);
    if (l.right - l.left < 24) bad.push(`${l.text}: link is only ${(l.right - l.left).toFixed(0)}px wide`);
    if (l.hidden) continue;
    drawn++;
    if (l.labelScroll > l.labelClient) bad.push(`${l.text}: label cut, scrollWidth ${l.labelScroll} > clientWidth ${l.labelClient}`);
    if (l.fontPx < MIN_FONT_PX) bad.push(`${l.text}: label is ${l.fontPx}px, under ${MIN_FONT_PX}px`);
    if (l.labelLeft < l.left - 0.5 || l.labelRight > l.right + 0.5) {
      bad.push(`${l.text}: label ${l.labelLeft.toFixed(0)}..${l.labelRight.toFixed(0)} spills out of its link ${l.left.toFixed(0)}..${l.right.toFixed(0)}`);
    }
  }
  // The active page is named on screen.
  const active = m.links.filter((l) => l.active);
  if (active.length !== 1) bad.push(`${active.length} active links`);
  else if (active[0].hidden) bad.push(`${active[0].text}: the active link shows no label`);
  if (drawn === 0) bad.push("no label is drawn at all");
  const sorted = [...m.links].sort((a, b) => a.left - b.left);
  for (let i = 1; i < sorted.length; i++) {
    if (sorted[i].left < sorted[i - 1].right - 0.5) bad.push(`${sorted[i - 1].text} and ${sorted[i].text} overlap`);
  }
  if (m.lowest > m.nav.top + 0.5) {
    bad.push(`page end (${m.lowestName}, bottom ${m.lowest.toFixed(0)}) is under the nav (top ${m.nav.top.toFixed(0)})`);
  }
  return bad;
}

async function main() {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  let browser;
  try {
    browser = await chromium.launch({ headless: true });
  } catch (e) {
    // No downloaded Chromium (a CI runner): the Chrome that comes with the runner will do.
    browser = await chromium.launch({ headless: true, channel: "chrome" });
  }
  const failures = [];
  let checks = 0;
  const wide = WIDTHS.filter((w) => w > 720);
  try {
    for (const width of WIDTHS) {
      for (const scheme of SCHEMES) {
        const ctx = await browser.newContext({
          viewport: { width, height: 844 },
          colorScheme: scheme,
          isMobile: width <= 720,
          hasTouch: width <= 720,
        });
        await ctx.addInitScript((s) => {
          try {
            localStorage.setItem("canvas-rgb.theme", s);
            // "Later" in this launch: the first-run onboarding would take the whole window and
            // draw no nav on the Layout page.
            sessionStorage.setItem("canvas-rgb.launch", "nav-check");
            localStorage.setItem("canvas-rgb.onboarding-later", "nav-check");
          } catch {
            /* storage blocked */
          }
        }, scheme);
        const page = await ctx.newPage();
        for (const p of PAGES) {
          await page.goto(`http://127.0.0.1:${port}${p.url}`, { waitUntil: "load" });
          await page.evaluate(() => document.fonts.ready);
          await page.waitForTimeout(700);
          await page.evaluate(() => window.scrollTo(0, 0));
          if (shotsDir) {
            fs.mkdirSync(shotsDir, { recursive: true });
            await page.screenshot({ path: path.join(shotsDir, `${shotTag}-${p.name}-${width}-${scheme}.png`) });
          }
          if (wide.includes(width)) continue; // desktop keeps its rail; only the shots are taken
          const m = await page.evaluate(MEASURE);
          checks++;
          const bad = judge(m, p, width);
          const summary = m.links
            ? m.links.map((l) => `${l.text}${l.hidden ? "(icon)" : ""}:${l.fontPx}px,${l.labelScroll}/${l.labelClient}`).join(" ")
            : "";
          const clear = m.nav ? `page end ${m.lowest.toFixed(0)} / nav top ${m.nav.top.toFixed(0)}` : "";
          console.log(`${bad.length ? "FAIL" : "ok  "} ${width} ${scheme} ${p.name}  [${clear}]  ${summary}`);
          for (const b of bad) {
            console.log(`       - ${b}`);
            failures.push(`${width} ${scheme} ${p.name}: ${b}`);
          }
        }
        await ctx.close();
      }
    }
  } finally {
    await browser.close();
    server.close();
  }
  console.log(`\n${checks} page checks, ${failures.length} failures`);
  if (checks === 0) process.exitCode = 1;
  if (failures.length) process.exitCode = 1;
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
