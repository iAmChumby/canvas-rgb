// The page bar, the status strip and the rail are measured in a real browser (issue #586).
//
// jsdom has no layout, so a unit test cannot see "Fan…" where "Fans" should be. This serves the
// static export (app/web/out) on loopback and drives it with headless Chromium. With no Tauri host
// the editor uses its own demo backend, so no daemon and no dev server are needed. The check
// walks the bar, the strip, the rail, the status popover and the command palette at seven sizes
// and in both colour schemes, and fails when a leaf of text is drawn shorter than its full text.
// Wrappers with no text of their own are skipped: the audit showed those outlines were false
// positives. SVG axis text inside .fan-curve is skipped by element, not by tag.
//
// Usage (after `npm --prefix app/web run build`):
//   node tools/bar-check.mjs                       exit 1 on any clip
//   node tools/bar-check.mjs --mutate-title        force a clipped title: this run must fail
//   node tools/bar-check.mjs --shots <dir>         also save a screenshot of every measured state
// Output: .build/bar-check/widths.json (needed and given widths, and the chosen form per state).

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "..");
const out = path.join(repo, "app", "web", "out");
const reportDir = path.join(repo, ".build", "bar-check");
const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  return i === -1 ? null : args[i + 1];
};
const shotsDir = flag("--shots");
const mutateTitle = args.includes("--mutate-title");
const MUTATE_CSS = ".rd-title h1 { max-width: 18px !important; }";

export const SIZES = [
  { w: 1920, h: 1040 },
  { w: 1440, h: 900 },
  { w: 1280, h: 800 },
  { w: 1024, h: 768 },
  { w: 768, h: 1024 },
  { w: 390, h: 844 },
  { w: 360, h: 780 },
];
export const ROUTES = [
  { name: "scenes", url: "/" },
  { name: "effects", url: "/effects/" },
  { name: "devices", url: "/devices/" },
  { name: "displays", url: "/displays/" },
  { name: "peripherals", url: "/peripherals/" },
  { name: "fans", url: "/fans/" },
  { name: "settings", url: "/settings/" },
  { name: "help", url: "/help/" },
  { name: "updates", url: "/updates/" },
  { name: "patterns", url: "/patterns/" },
];
// A 60-character name a person typed: the builder's h1 gives way, Save never does (#586 R1).
export const LONG_NAME = "A very long stack name that somebody typed in the builder by hand";
export const STATES = [
  ...ROUTES.map((r) => ({ ...r, state: "loaded" })),
  // R11: Updates and Settings draw PageTitle while loading and again once loaded.
  { name: "updates", url: "/updates/", state: "loading" },
  { name: "settings", url: "/settings/", state: "loading" },
  // R1: the user-named title, with every conditional item present: dirty, pending, group open.
  { name: "builder", url: `/effects/?stack=${encodeURIComponent(LONG_NAME)}&group=Light%20setup`, state: "long-name" },
  // R11: the palette and the popover have no bar of their own; only clipped text is checked.
  { name: "command-palette", url: "/fans/", state: "palette" },
  { name: "status-popover", url: "/fans/", state: "popover" },
];
const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".ico": "image/x-icon",
  ".txt": "text/plain; charset=utf-8",
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
  res.writeHead(200, {
    "content-type": MIME[path.extname(file)] ?? "application/octet-stream",
    "cache-control": "no-store",
  });
  fs.createReadStream(file).pipe(res);
});

/** What the bar looks like now. Runs inside the page. */
export const MEASURE = () => {
  const norm = (s) => (s ?? "").replace(/\s+/g, " ").trim();
  const label = (el) => {
    const cls = String(el.getAttribute("class") ?? "").trim().split(/\s+/).filter(Boolean).slice(0, 3).map((c) => `.${c}`).join("");
    return `${el.tagName.toLowerCase()}${cls}`;
  };
  const drawn = (el) => {
    const cs = getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden" || Number(cs.opacity) === 0) return false;
    const r = el.getBoundingClientRect();
    return r.width > 1 && r.height > 1;
  };
  const ownText = (el) => norm([...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.nodeValue).join(" "));
  const clip = [];
  const scopes = [".strip-inner", ".rail", ".strip-details", ".cmdk", "main .select-trigger"];
  const seen = new Set();
  for (const scope of scopes) {
    for (const root of document.querySelectorAll(scope)) {
      for (const el of [root, ...root.querySelectorAll("*")]) {
        if (seen.has(el) || el.closest(".fan-curve")) continue;
        seen.add(el);
        if (!drawn(el)) continue;
        const text = ownText(el);
        if (!text) continue;
        const need = el.scrollWidth;
        const given = el.clientWidth;
        if (need > given) {
          const userTitle = el.hasAttribute("data-bar-user-title") || el.closest("[data-bar-user-title]") !== null;
          const carrier = el.getAttribute("title") || el.closest("[title]")?.getAttribute("title") || el.getAttribute("aria-label") || "";
          clip.push({
            selector: label(el),
            text,
            full: carrier || norm(el.textContent),
            need,
            given,
            primary: el.hasAttribute("data-bar-primary") || el.closest("[data-bar-primary]") !== null,
            title: el.matches(".rd-title h1") || userTitle,
            userTitle,
            inBar: el.closest(".strip-inner") !== null,
            inRail: el.closest(".rail") !== null,
            overflow: getComputedStyle(el).textOverflow,
          });
        }
      }
    }
  }
  // The visible forms, so widths.json records what the priority rules chose at each size.
  const shown = (sel) => {
    const el = document.querySelector(sel);
    return el ? drawn(el) : false;
  };
  const box = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { w: Math.round(r.width), need: el.scrollWidth, given: el.clientWidth };
  };
  // A control whose accessible name is shorter than the words it draws loses those words for a
  // screen reader (the Settings audio-source trigger did: "Audio source" over a long value).
  const names = [];
  // The path filter is the shell and the strip stylesheet: a select in a page body is out of
  // scope, apart from the audio-source picker (#586 R8), whose value used to be cut.
  for (const el of document.querySelectorAll(".strip-inner [aria-label], .rail [aria-label], .audio-picker .select-trigger[aria-label]")) {
    if (!drawn(el)) continue;
    const name = norm(el.getAttribute("aria-label"));
    if (el.matches(".audio-picker .select-trigger[aria-label]")) {
      // The trigger's value is in a child; compare the chosen value, not every hidden option.
      const chosen = norm(el.querySelector('.select-value [data-on="true"]')?.textContent);
      if (chosen && !name.includes(chosen)) names.push({ selector: label(el), visible: chosen, name });
      continue;
    }
    const visible = ownText(el);
    if (visible.length > name.length + 1) names.push({ selector: label(el), visible, name });
  }
  return {
    vw: document.documentElement.clientWidth,
    vh: document.documentElement.clientHeight,
    names,
    primaryPresent: document.querySelector(".strip-inner [data-bar-primary]") !== null || document.querySelector(".rd-title") === null,
    userTitlePresent: document.querySelector("[data-bar-user-title]") !== null || document.querySelector(".rd-stacks") === null,
    stripW: box(".strip-inner")?.w ?? null,
    barRows: document.querySelector(".strip")?.getAttribute("data-bar-rows") ?? null,
    title: box(".rd-title h1"),
    primary: box("[data-bar-primary]"),
    rail: box(".rail-value"),
    forms: {
      context: shown(".rd-context"),
      nowName: shown(".strip-effect-name"),
      nowDetail: shown(".strip-now-detail"),
      markWords: shown(".strip-mark-text"),
      segmented: shown(".rd-actions [role='radiogroup']"),
      powerWords: shown(".strip-power-text"),
    },
    clip,
  };
};

/**
 * A synthetic measurement is judged exactly like a browser one. Exported so tools/tests can
 * check the rules without a browser. A clip of a title, a primary action or any leaf text is a
 * failure; a user-named title is allowed to give way, but never below its 12-character floor.
 */
export const USER_TITLE_FLOOR = 12;

export function judge(m, state) {
  const bad = [];
  const where = `${state.name} ${state.state}`;
  if (!m) return [`${where}: no measurement`];
  if (m.docOverflow > 0) bad.push(`${where}: page scrolls sideways by ${m.docOverflow}px`);
  for (const c of m.clip) {
    // R1: a name a person typed gives way in the middle. It may clip, but only while its full
    // words stay in title/aria-label and at least twelve characters are still drawn.
    if (c.userTitle) {
      if (c.text.length < USER_TITLE_FLOOR) bad.push(`${where}: user title dropped to ${c.text.length} chars, floor ${USER_TITLE_FLOOR}`);
      if (!c.full || !c.full.includes(c.text.replace(/…/g, "").slice(0, 8))) bad.push(`${where}: user title has no full name in its title or accessible name`);
      continue;
    }
    const kind = c.title ? "title" : c.primary ? "primary action" : c.inRail ? "rail value" : "text";
    bad.push(`${where}: ${kind} ${c.selector} "${c.text}" needs ${c.need}px, given ${c.given}px`);
  }
  for (const n of m.names ?? []) {
    bad.push(`${where}: ${n.selector} accessible name "${n.name}" is shorter than its visible words "${n.visible}"`);
  }
  // Only these pages put a primary action in the bar. Scenes has secondary keys only; a page
  // that has no primary key is not required to invent one.
  if (["fans", "effects", "builder"].includes(state.name) && m.primaryPresent === false) {
    bad.push(`${where}: the primary action left the bar`);
  }
  if (state.state === "long-name" && m.userTitlePresent === false) bad.push(`${where}: the user-named title left the bar`);
  return bad;
}

async function main() {
  if (!fs.existsSync(path.join(out, "index.html"))) {
    throw new Error("static export not found; run npm --prefix app/web run build first");
  }
  const require = createRequire(path.join(repo, "tools", "poison-compare", "js", "package.json"));
  const { chromium } = require("playwright");
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  const base = `http://127.0.0.1:${port}`;
  let browser;
  let channel = "chromium";
  try {
    browser = await chromium.launch({ headless: true });
  } catch {
    channel = "chrome";
    browser = await chromium.launch({ headless: true, channel: "chrome" });
  }
  console.log(`bar-check: browser channel ${channel}`);
  if (mutateTitle) console.log("bar-check: MUTATION on, a clipped title is forced: this run must fail");
  const failures = [];
  const widths = [];
  let checks = 0;
  try {
    for (const size of SIZES) {
      for (const scheme of ["dark", "light"]) {
        const ctx = await browser.newContext({
          viewport: { width: size.w, height: size.h },
          colorScheme: scheme,
          deviceScaleFactor: 1,
          isMobile: size.w <= 720,
          hasTouch: size.w <= 720,
        });
        await ctx.addInitScript((s) => {
          try {
            localStorage.setItem("canvas-rgb.theme", s);
            // The first-run onboarding and the header-setup modal would cover the bar.
            sessionStorage.setItem("canvas-rgb.launch", "bar-check");
            localStorage.setItem("canvas-rgb.onboarding-later", "bar-check");
            localStorage.setItem("canvas-rgb.chains-later", "bar-check");
          } catch {
            /* storage blocked */
          }
        }, scheme);
        const page = await ctx.newPage();
        for (const state of STATES) {
          await page.goto(base + state.url, { waitUntil: "load" });
          await page.evaluate(() => document.fonts.ready);
          if (state.state === "loading") {
            // Measured before the demo backend answers: both states draw PageTitle (R11).
            const m = await page.evaluate(MEASURE);
            checks++;
            widths.push({ page: state.name, state: state.state, size: `${size.w}x${size.h}`, theme: scheme, ...m });
            const bad = judge({ ...m, docOverflow: 0 }, state);
            for (const b of bad) {
              console.error(`FAIL ${size.w}x${size.h} ${scheme} ${b}`);
              failures.push(`${size.w}x${size.h} ${scheme} ${b}`);
            }
          }
          await page.waitForFunction(() => document.querySelector('[data-live="on"]') !== null, { timeout: 10000 }).catch(() => {});
          await page.waitForTimeout(450);
          if (state.state === "palette") {
            await page.keyboard.press("Control+k");
            await page.waitForSelector(".cmdk", { timeout: 10000 }).catch(() => {});
          } else if (state.state === "popover") {
            await page.locator(".strip-mark").first().click().catch(() => {});
            await page.waitForSelector(".strip-details", { timeout: 10000 }).catch(() => {});
          }
          await page.waitForTimeout(200);
          if (mutateTitle) await page.addStyleTag({ content: MUTATE_CSS });
          if (shotsDir) {
            fs.mkdirSync(shotsDir, { recursive: true });
            const tag = `${state.name}-${state.state}-${size.w}x${size.h}-${scheme}`;
            await page.screenshot({ path: path.join(shotsDir, `${tag}.png`) });
          }
          const m = await page.evaluate(MEASURE);
          checks++;
          widths.push({ page: state.name, state: state.state, size: `${size.w}x${size.h}`, theme: scheme, ...m });
          const bad = judge({ ...m, docOverflow: 0 }, state);
          const summary = `strip ${m.stripW}px, title ${m.title?.w ?? "-"}px, primary ${m.primary?.w ?? "-"}px, clips ${m.clip.length}`;
          console.log(`${bad.length ? "FAIL" : "ok  "} ${size.w}x${size.h} ${scheme} ${state.name}/${state.state}  [${summary}]`);
          for (const b of bad) {
            console.error(`       - ${b}`);
            failures.push(`${size.w}x${size.h} ${scheme} ${b}`);
          }
          if (mutateTitle) await page.evaluate(() => document.querySelectorAll("style").forEach((s) => { if (s.textContent.includes("max-width: 18px")) s.remove(); }));
        }
        await ctx.close();
      }
    }
  } finally {
    await browser.close();
    server.close();
  }
  fs.mkdirSync(reportDir, { recursive: true });
  fs.writeFileSync(path.join(reportDir, "widths.json"), JSON.stringify(widths, null, 2));
  console.log(`\n${checks} measurements, ${failures.length} failures; widths: .build/bar-check/widths.json`);
  if (failures.length || checks === 0) process.exitCode = 1;
  if (mutateTitle && failures.length === 0) {
    console.error("bar-check: the mutation did not fail the check");
    process.exitCode = 1;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((e) => {
    console.error(e);
    process.exit(2);
  });
}
