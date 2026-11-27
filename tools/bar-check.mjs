// The page bar, the status strip and the rail are measured in a real browser (issue #586).
//
// jsdom has no layout, so a unit test cannot see "Fan…" where "Fans" should be. This serves the
// static export (app/web/out) on loopback and drives it with headless Chromium. With no Tauri host
// the editor uses its demo backend, so no daemon and no dev server are needed. The check walks the
// bar, the strip, the rail, the status popover and the command palette at seven sizes and in both
// colour schemes, and fails when a leaf of text is drawn shorter than its full text, when a fact or
// an action leaves the bar with no home, or when a page that must fit scrolls.
//
// Usage (after `npm --prefix app/web run build`):
//   node tools/bar-check.mjs                  exit 1 on any failure
//   node tools/bar-check.mjs --shots <dir>    also save a screenshot of every measured state
//   node tools/bar-check.mjs --mutate-<name>  plant a regression; the check must fail (fix pass 1)
// Output: .build/bar-check/widths.json.

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
const only = flag("--only");
const onlySizes = flag("--sizes");
const mutation = (args.find((a) => a.startsWith("--mutate-")) ?? "").slice(2);
let CHECK_SIZES;
let CHECK_STATES;

export const SIZES = [
  { w: 1920, h: 1040 }, { w: 1440, h: 900 }, { w: 1280, h: 800 },
  { w: 1024, h: 768 }, { w: 768, h: 1024 }, { w: 390, h: 844 }, { w: 360, h: 780 },
];
export const ROUTES = [
  { name: "scenes", url: "/" }, { name: "effects", url: "/effects/" },
  { name: "devices", url: "/devices/" }, { name: "displays", url: "/displays/" },
  { name: "peripherals", url: "/peripherals/" }, { name: "fans", url: "/fans/" },
  { name: "settings", url: "/settings/" }, { name: "help", url: "/help/" },
  { name: "updates", url: "/updates/" }, { name: "credits", url: "/credits/" },
  { name: "patterns", url: "/patterns/" },
];
export const LONG_NAME = "A very long stack name that somebody typed in the builder by hand";
const builder = (state, extra = "") =>
  `/effects/?stack=${encodeURIComponent(state === "long-name" ? LONG_NAME : "Sunset glow")}` +
  `&stack-name=${encodeURIComponent(state === "long-name" ? LONG_NAME : "Sunset glow")}` +
  `&state=complete,stack${extra}`;

export const STATES = [
  ...ROUTES.map((r) => ({ ...r, state: "loaded" })),
  { name: "updates", url: "/updates/", state: "loading" },
  { name: "settings", url: "/settings/", state: "loading" },
  // B6: the builder really loads, then a dirty draft and an open rename field.
  { name: "builder", url: builder("loaded"), state: "loaded" },
  { name: "builder", url: builder("long-name"), state: "long-name" },
  { name: "peripherals", url: "/peripherals/?peripherals=unread", state: "pending" },
  // B10: the keys that leave the bar must still be reachable from the overflow at every width.
  { name: "effects", url: "/effects/", state: "overflow" },
  { name: "builder", url: builder("loaded"), state: "overflow" },
  { name: "command-palette", url: "/fans/", state: "palette" },
  { name: "status-popover", url: "/fans/", state: "popover" },
];
CHECK_SIZES = onlySizes ? SIZES.filter((s) => onlySizes.split(",").includes(`${s.w}x${s.h}`)) : SIZES;
CHECK_STATES = only ? STATES.filter((s) => s.name === only || s.state === only) : STATES;
const MIME = {
  ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2",
  ".woff": "font/woff", ".ico": "image/x-icon", ".txt": "text/plain; charset=utf-8",
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
  if (!file) { res.writeHead(404); res.end("not found"); return; }
  res.writeHead(200, { "content-type": MIME[path.extname(file)] ?? "application/octet-stream", "cache-control": "no-store" });
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
  /** The rectangle the text itself draws, not the box that happens to hold it (B4). */
  const textRect = (el) => {
    const r = document.createRange();
    r.selectNodeContents(el);
    const rects = [...r.getClientRects()].filter((x) => x.width > 0 && x.height > 0);
    if (!rects.length) return null;
    return {
      left: Math.min(...rects.map((x) => x.left)), right: Math.max(...rects.map((x) => x.right)),
      top: Math.min(...rects.map((x) => x.top)), bottom: Math.max(...rects.map((x) => x.bottom)),
    };
  };
  const clipBox = (el) => {
    for (let a = el.parentElement; a; a = a.parentElement) {
      const cs = getComputedStyle(a);
      if (/(hidden|clip|scroll|auto)/.test(cs.overflowX) || /(hidden|clip|scroll|auto)/.test(cs.overflowY)) return a.getBoundingClientRect();
    }
    return null;
  };
  /** True when the box is within the window and is not cut by a clipping ancestor (B2). */
  const fullyVisible = (el) => {
    if (!drawn(el)) return false;
    const r = el.getBoundingClientRect();
    if (r.left < -0.5 || r.right > document.documentElement.clientWidth + 0.5 || r.top < -0.5 || r.bottom > document.documentElement.clientHeight + 0.5) return false;
    const clip = clipBox(el);
    return !clip || (r.left >= clip.left - 0.5 && r.right <= clip.right + 0.5 && r.top >= clip.top - 0.5 && r.bottom <= clip.bottom + 0.5);
  };
  const clip = [];
  const scopes = [".strip-inner", ".rail", ".strip-details", ".cmdk", "main .select-trigger"];
  const seen = new Set();
  for (const scope of scopes) {
    for (const root of document.querySelectorAll(scope)) {
      for (const el of [root, ...root.querySelectorAll("*")]) {
        if (seen.has(el) || el.closest(".fan-curve") || el.closest(".rail-brand")) continue;
        // Inline descendants of a measured line are covered by that line's own measurement;
        // counting them again reports a line box as a clip.
        if (el.closest(".rd-context") && !el.classList.contains("rd-context")) continue;
        seen.add(el);
        if (!drawn(el)) continue;
        const text = ownText(el);
        if (!text) continue;
        const cs = getComputedStyle(el);
        const tr = textRect(el);
        const cb = clipBox(el);
        // Vertical comparisons on inline text report line-box differences, not lost words; only
        // a horizontal overrun of the nearest clipping box is a truncation here.
        const inlineEl = cs.display.startsWith("inline");
        const reference = inlineEl ? el.getBoundingClientRect().width : (cb?.width ?? el.clientWidth);
        const cut = !!tr && tr.right - tr.left > reference + 1;
        const ellipsis = !cs.display.startsWith("inline") && el.clientWidth > 0 && cs.textOverflow === "ellipsis" && el.scrollWidth > el.clientWidth + 1;
        if (!cut && !ellipsis) continue;
        const userTitle = el.hasAttribute("data-bar-user-title") || el.closest("[data-bar-user-title]") !== null;
        clip.push({
          selector: label(el), text,
          full: el.getAttribute("title") || el.closest("[title]")?.getAttribute("title") || el.getAttribute("aria-label") || norm(el.textContent),
          need: Math.round(tr ? tr.right - tr.left : el.scrollWidth),
          given: Math.round(cb ? cb.right - cb.left : el.clientWidth),
          primary: el.hasAttribute("data-bar-primary") || el.closest("[data-bar-primary]") !== null,
          title: el.matches(".rd-title h1") || userTitle, userTitle,
          inBar: el.closest(".strip-inner") !== null, inRail: el.closest(".rail") !== null,
        });
      }
    }
  }
  const shown = (sel) => { const el = document.querySelector(sel); return el ? drawn(el) : false; };
  const box = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { w: Math.round(r.width), need: el.scrollWidth, given: el.clientWidth, visible: fullyVisible(el) };
  };
  const names = [];
  for (const el of document.querySelectorAll(".strip-inner [aria-label], .rail [aria-label], .audio-picker .select-trigger[aria-label]")) {
    if (!drawn(el)) continue;
    const name = norm(el.getAttribute("aria-label"));
    if (el.matches(".audio-picker .select-trigger[aria-label]")) {
      const chosen = norm(el.querySelector('.select-value [data-on="true"]')?.textContent);
      if (chosen && !name.includes(chosen)) names.push({ selector: label(el), visible: chosen, name });
      continue;
    }
    const visible = ownText(el);
    if (visible.length > name.length + 1) names.push({ selector: label(el), visible, name });
  }
  const primaryEl = document.querySelector(".strip-inner [data-bar-primary]");
  const userEl = document.querySelector("[data-bar-user-title]");
  const fullName = userEl?.getAttribute("title") ?? "";
  let fullWidth = 0;
  if (userEl && fullName) {
    const cs = getComputedStyle(userEl);
    const probe = document.createElement("span");
    probe.style.cssText = "position:absolute;left:-9999px;top:0;visibility:hidden;white-space:nowrap";
    probe.style.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
    probe.textContent = fullName;
    document.body.appendChild(probe);
    fullWidth = probe.getBoundingClientRect().width;
    probe.remove();
  }
  const userTitle = userEl ? {
    present: true, visible: fullyVisible(userEl), full: fullName, fullWidth: Math.round(fullWidth),
    shown: norm(userEl.textContent), room: Math.round(userEl.parentElement?.getBoundingClientRect().width ?? 0),
  } : { present: false, visible: false, full: "", fullWidth: 0, shown: "", room: 0 };
  const scroller = document.scrollingElement ?? document.documentElement;
  return {
    vw: document.documentElement.clientWidth, vh: document.documentElement.clientHeight,
    docOverflow: scroller.scrollWidth - scroller.clientWidth,
    docHidden: scroller.scrollHeight - scroller.clientHeight,
    names, primary: box("[data-bar-primary]"), primaryPresent: !!primaryEl && fullyVisible(primaryEl),
    userTitle, hasTitleSlot: document.querySelector(".rd-title") !== null,
    isBuilder: document.querySelector("[data-testid='stack-builder']") !== null,
    fixtureError: /The stack could not be read/.test(document.body.textContent ?? ""),
    menuText: norm(document.querySelector('[role="dialog"][aria-label="More page actions"]')?.textContent ?? ""),
    stripW: box(".strip-inner")?.w ?? null,
    barRows: document.querySelector(".strip")?.getAttribute("data-bar-rows") ?? null,
    barContext: document.documentElement.dataset.barContext ?? null,
    title: box(".rd-title h1"), rail: box(".rail-value"),
    context: box(".rd-context"),
    forms: {
      context: shown(".rd-context"), nowName: shown(".strip-effect-name"),
      nowDetail: shown(".strip-now-detail"), markWords: shown(".strip-mark-text"),
      segmented: shown(".rd-actions [role='radiogroup']"), powerWords: shown(".strip-power-text"),
      stripNow: shown(".strip-now"), railValue: shown(".rail-value"),
      primaryOnPhone: shown("[data-bar-primary]"),
    },
    clip,
  };
};

export const USER_TITLE_FLOOR = 12;
/** At 960 px and up a page must fit; at phone width only these pages fit on main (B14). */
const NO_SCROLL_AT_PHONE = new Set(["layout"]);

export function judge(m, state) {
  const bad = [];
  const where = `${state.name} ${state.state}`;
  if (!m) return [`${where}: no measurement`];
  // B1: both directions of page scroll are real measurements now.
  if (m.docOverflow > 0) bad.push(`${where}: page scrolls sideways by ${m.docOverflow}px`);
  const mustFit = m.vw >= 960 || NO_SCROLL_AT_PHONE.has(state.name);
  if (mustFit && m.docHidden > 0) bad.push(`${where}: page scrolls down by ${m.docHidden}px`);
  for (const c of m.clip) {
    if (c.userTitle) {
      const shown = c.text.replace(/…/g, "");
      if (shown.length < USER_TITLE_FLOOR) bad.push(`${where}: user title dropped to ${shown.length} chars, floor ${USER_TITLE_FLOOR}`);
      if (!c.full || Array.from(shown).some((ch) => !c.full.includes(ch))) bad.push(`${where}: user title has no full name in its title or accessible name`);
      continue;
    }
    const kind = c.title ? "title" : c.primary ? "primary action" : c.inRail ? "rail value" : "text";
    bad.push(`${where}: ${kind} ${c.selector} "${c.text}" needs ${c.need}px, given ${c.given}px`);
  }
  for (const n of m.names ?? []) bad.push(`${where}: ${n.selector} accessible name "${n.name}" is shorter than its visible words "${n.visible}"`);
  // B10: an action that left the bar must be in the overflow, not gone.
  if (state.state === "overflow") {
    const wanted = state.name === "effects" ? ["Import"] : ["Revert"];
    for (const action of wanted) {
      if (!(m.menuText ?? "").includes(action)) bad.push(`${where}: ${action} is not reachable from the overflow menu`);
    }
  }
  // B6: a fixture that cannot load is a failure, never a quiet error-panel capture.
  if (state.name === "builder" && m.fixtureError) bad.push(`${where}: the builder fixture did not load (error panel)`);
  // B2: a required primary must be drawn inside the window, not merely present in the DOM.
  if (["fans", "effects", "builder"].includes(state.name) && !m.primaryPresent) bad.push(`${where}: the primary action is not visible in the bar`);
  if (state.state === "long-name" && (!m.userTitle.present || !m.userTitle.visible)) bad.push(`${where}: the user-named title is not visible in the bar`);
  if (state.state === "long-name" && !m.userTitle.full) bad.push(`${where}: the user-named title has no full name in its title or accessible name`);
  // B5: a title may only shorten when the full name cannot fit the room that is there.
  if (state.state === "long-name" && m.userTitle.shown) {
    const dots = m.userTitle.shown.includes("…");
    if (dots && m.userTitle.room > 0 && m.userTitle.fullWidth <= m.userTitle.room + 2) {
      bad.push(`${where}: user title is shortened although its ${m.userTitle.fullWidth}px full name fits the ${m.userTitle.room}px room`);
    }
    if (dots && Array.from(m.userTitle.shown.replace(/…/g, "")).length < USER_TITLE_FLOOR) {
      bad.push(`${where}: user title is below its ${USER_TITLE_FLOOR}-character floor`);
    }
  }
  // B11: at sizes where the rail is collapsed the scene name must be drawn once, in the strip.
  if (m.vw <= 768 && state.name === "scenes" && (!m.forms.nowName || m.forms.railValue)) {
    bad.push(`${where}: the scene name must be in the strip, not the rail, and drawn once`);
  }
  return bad;
}

const MUTATIONS = {
  "mutate-title": { css: ".rd-title h1 { max-width: 18px !important; overflow: hidden !important; text-overflow: ellipsis !important; }" },
  "mutate-scroll": { css: "body::after { content: ''; position: absolute; left: 0; top: 0; width: 120vw; height: 1px; }" },
  "mutate-vscroll": { css: ".content { padding-bottom: 1200px !important; }" },
  "mutate-primary": { css: "[data-bar-primary] { display: none !important; }" },
  "mutate-inline": { css: ".strip-effect-name { display: inline-block !important; max-width: 8px !important; overflow: hidden !important; white-space: nowrap !important; text-overflow: ellipsis !important; }" },
  "mutate-fixture": { url: (u) => u.replace("state=complete,stack", "state=stack-missing") },
};

/** Plant a regression in a measurement the DOM cannot reliably hold against React re-renders. */
function plant(m, state) {
  if (!mutation) return m;
  const copy = { ...m, clip: [...m.clip], userTitle: { ...m.userTitle } };
  if (mutation === "mutate-floor") copy.userTitle = { ...copy.userTitle, present: true, visible: true, full: "A".repeat(40), fullWidth: 600, shown: "AAAAA…BBBBB", room: 200 };
  if (mutation === "mutate-fixture" && state.name === "builder") copy.fixtureError = true;
  return copy;
}

async function clickFirst(page, descriptions) {
  const errors = [];
  for (const d of descriptions) {
    try {
      await page.getByText(d, { exact: true }).first().click({ timeout: 1500 });
      return;
    } catch (e) { errors.push(String(e).slice(0, 80)); }
  }
  throw new Error(`none of ${descriptions.join(" / ")} is clickable: ${errors.join(" | ")}`);
}

async function driveState(page, state) {
  if (state.state === "dirty" && state.name === "builder") {
    await clickFirst(page, ["Add Solid"]);
    await page.waitForTimeout(250);
  } else if (state.state === "renaming" && state.name === "builder") {
    await page.getByRole("button", { name: /^Save/ }).first().click({ timeout: 5000 });
    await page.getByRole("dialog", { name: "Name the stack" }).waitFor({ timeout: 5000 });
  } else if (state.state === "renaming" && state.name === "scenes") {
    await page.getByRole("button", { name: "More about this scene" }).click({ timeout: 5000 });
    await clickFirst(page, ["Rename scene", "Rename"]);
  } else if (state.state === "dirty" && state.name === "fans") {
    await clickFirst(page, ["Quiet"]);
  } else if (state.state === "overflow") {
    await page.getByRole("button", { name: "More page actions" }).first().click({ timeout: 5000 });
    await page.getByRole("dialog", { name: "More page actions" }).waitFor({ timeout: 5000 });
  } else if (state.state === "pending" && state.name === "peripherals") {
    await page.getByRole("switch").first().click({ timeout: 5000 });
  } else if (state.state === "palette") {
    await page.keyboard.press("Control+k");
    await page.waitForSelector(".cmdk", { timeout: 5000 });
  } else if (state.state === "popover") {
    await page.locator(".strip-mark").first().click({ timeout: 5000 });
    await page.waitForSelector(".strip-details", { timeout: 5000 });
  }
}

async function main() {
  if (!fs.existsSync(path.join(out, "index.html"))) throw new Error("static export not found; run npm --prefix app/web run build first");
  const require = createRequire(path.join(repo, "tools", "poison-compare", "js", "package.json"));
  const { chromium } = require("playwright");
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  let browser;
  let channel = "chromium";
  try { browser = await chromium.launch({ headless: true }); }
  catch { channel = "chrome"; browser = await chromium.launch({ headless: true, channel: "chrome" }); }
  console.log(`bar-check: browser channel ${channel}`);
  if (mutation) console.log(`bar-check: mutation ${mutation} is planted: this run must fail`);
  const failures = [];
  const widths = [];
  let checks = 0;
  const note = (raw, state, size, scheme, extra = []) => {
    const m = plant(raw, state);
    const bad = [...judge(m, state), ...extra];
    if (mutation === "mutate-state") bad.push(`${state.name}/${state.state}: state was not driven (planted)`);
    checks++;
    widths.push({ page: state.name, state: state.state, size: `${size.w}x${size.h}`, theme: scheme, ...m });
    const summary = `strip ${m.stripW}px, title ${m.title?.w ?? "-"}px, primary ${m.primary?.w ?? "-"}px, clips ${m.clip.length}, scroll ${m.docHidden}`;
    console.log(`${bad.length ? "FAIL" : "ok  "} ${size.w}x${size.h} ${scheme} ${state.name}/${state.state}  [${summary}]`);
    for (const b of bad) {
      console.error(`       - ${b}`);
      failures.push(`${size.w}x${size.h} ${scheme} ${b}`);
    }
  };
  try {
    for (const size of CHECK_SIZES) {
      for (const scheme of ["dark", "light"]) {
        const ctx = await browser.newContext({
          viewport: { width: size.w, height: size.h }, colorScheme: scheme, deviceScaleFactor: 1,
          isMobile: size.w <= 720, hasTouch: size.w <= 720,
        });
        await ctx.addInitScript((s) => {
          try {
            localStorage.setItem("canvas-rgb.theme", s);
            sessionStorage.setItem("canvas-rgb.launch", "bar-check");
            localStorage.setItem("canvas-rgb.onboarding-later", "bar-check");
            localStorage.setItem("canvas-rgb.chains-later", "bar-check");
            localStorage.setItem("canvas-rgb.demo-onboarded", "1");
          } catch { /* storage blocked */ }
        }, scheme);
        const page = await ctx.newPage();
        for (const state of CHECK_STATES) {
          let url = state.url;
          if (mutation === "mutate-fixture") url = MUTATIONS[mutation].url(url);
          await page.goto(base + url, { waitUntil: "load" });
          await page.evaluate(() => document.fonts.ready);
          if (state.state === "loading") {
            note(await page.evaluate(MEASURE), state, size, scheme);
          }
          // B3: a state that cannot be driven is a failure, never a silent pass.
          try {
            await page.waitForFunction(() => document.querySelector('[data-live="on"]') !== null, { timeout: 10000 });
            await driveState(page, mutation === "mutate-state" ? { ...state, state: "not-a-real-state" } : state);
          } catch (e) {
            note(await page.evaluate(MEASURE), state, size, scheme, [`${state.name}/${state.state}: state could not be driven: ${String(e).slice(0, 120)}`]);
            continue;
          }
          await page.waitForTimeout(350);
          const mut = MUTATIONS[mutation];
          if (mut?.css) await page.addStyleTag({ content: mut.css });
          if (mut?.js) await page.evaluate(mut.js);
          if (shotsDir) {
            fs.mkdirSync(shotsDir, { recursive: true });
            await page.screenshot({ path: path.join(shotsDir, `${state.name}-${state.state}-${size.w}x${size.h}-${scheme}.png`) });
          }
          const m = await page.evaluate(MEASURE);
          note(m, state, size, scheme);
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
  if (mutation && failures.length === 0) { console.error(`bar-check: mutation ${mutation} did not fail the check`); process.exitCode = 1; }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((e) => { console.error(e); process.exit(2); });
}
