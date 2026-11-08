// Probe React's real boxes for the onboarding takeover (#212).
// Headless Chromium, demo backend (no __TAURI__), ?state= empty (happy Hello).
// Dumps getBoundingClientRect + computed font-size/line-height/weight/padding/gap/border
// for each step's elements at 1440x900 dark.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(new URL("../poison-compare/js/package.json", import.meta.url));
const { chromium } = require("playwright");

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "../..");
const CAPTURE_PORT = 4312;
const WEB_OUT = path.resolve(root, "app/web/out");

const MIME = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".json": "application/json", ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2" };

function serveStatic(port, dir) {
  const server = http.createServer((req, res) => {
    try {
      const url = new URL(req.url ?? "/", `http://127.0.0.1:${port}`);
      let pathname = decodeURIComponent(url.pathname);
      let file = path.normalize(path.join(dir, pathname));
      if (!file.startsWith(dir)) { res.writeHead(403); res.end("forbidden"); return; }
      if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, "index.html");
      if (!fs.existsSync(file) && !path.extname(pathname)) {
        const alt = path.join(dir, pathname, "index.html");
        if (fs.existsSync(alt)) file = alt;
      }
      if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) { res.writeHead(404); res.end("not found"); return; }
      const ext = path.extname(file).toLowerCase();
      res.writeHead(200, { "Content-Type": MIME[ext] ?? "application/octet-stream", "Cache-Control": "no-store" });
      fs.createReadStream(file).pipe(res);
    } catch (e) { res.writeHead(500); res.end(String(e)); }
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve(server));
  });
}

async function probe(browser, base, step, out) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  // Demo mode: no __TAURI__.
  await page.goto(`${base}/`, { waitUntil: "load", timeout: 30000 });
  try { await page.waitForFunction(() => document.querySelector('[data-live="on"]') !== null, { timeout: 8000 }); } catch {}
  // Advance by clicking the primary button by its label (CSS-module classes are hashed).
  const clickPrimary = async () => {
    const labels = ["Map my PC", "Looks right", "Match the lights", "Nothing's lit", "Continue", "Done", "Yes", "That's all of it"];
    for (const b of await page.$$("button")) {
      const text = (await b.textContent())?.trim() ?? "";
      if (labels.some((l) => text === l)) {
        await b.click();
        await page.waitForTimeout(500);
        return true;
      }
    }
    return false;
  };
  const currentStep = async () => await page.evaluate(() => document.querySelector('[data-step]')?.getAttribute('data-step') ?? null);
  for (let i = 0; i < 12; i++) {
    if ((await currentStep()) === step) break;
    if (!(await clickPrimary())) break;
  }
  await page.waitForTimeout(600);
  const data = await page.evaluate(() => {
    const pick = (sel) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      return {
        rect: { x: r.x, y: r.y, w: r.width, h: r.height },
        fontSize: cs.fontSize, lineHeight: cs.lineHeight, fontWeight: cs.fontWeight,
        paddingTop: cs.paddingTop, paddingRight: cs.paddingRight, paddingBottom: cs.paddingBottom, paddingLeft: cs.paddingLeft,
        gap: cs.gap, borderTopWidth: cs.borderTopWidth, borderLeftWidth: cs.borderLeftWidth,
        borderTopColor: cs.borderTopColor, backgroundColor: cs.backgroundColor, color: cs.color,
      };
    };
    // CSS-module classes are hashed; find the onboarding root by its data-step.
    const root = document.querySelector('[data-step]');
    const bar = root ? root.querySelector('header') : document.querySelector('header');
    const panel = root ? root.querySelector('section') : null;
    const title = panel ? panel.querySelector('h2') : document.querySelector('h2');
    const lead = document.querySelector('[data-step] section p');
    const kicker = panel ? panel.querySelector('div') : null;
    const cta = root ? root.querySelector('button[class*="cta"]') ?? root.querySelector('button.btn-primary') : null;
    const main = root ? root.querySelector('[class*="main"]') : null;
    const greeting = document.querySelector('h1');
    const count = root ? root.querySelector('[aria-live="polite"]') : null;
    const dots = document.querySelector('[role="progressbar"]');
    const truth = document.querySelector('[role="status"]');
    const size = panel ? panel.querySelector('[aria-live="polite"]') : null;
    const tiles = panel ? panel.querySelectorAll('button[aria-pressed]') : [];
    const out = {};
    const grab = (key, el) => {
      if (!el) { out[key] = null; return; }
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      out[key] = {
        rect: { x: r.x, y: r.y, w: r.width, h: r.height },
        fontSize: cs.fontSize, lineHeight: cs.lineHeight, fontWeight: cs.fontWeight,
        padding: `${cs.paddingTop} ${cs.paddingRight} ${cs.paddingBottom} ${cs.paddingLeft}`,
        gap: cs.gap, border: `${cs.borderTopWidth} / ${cs.borderLeftWidth}`,
        color: cs.color, bg: cs.backgroundColor,
      };
    };
    grab("bar", bar); grab("panel", panel); grab("main", main); grab("title", title);
    grab("lead", lead); grab("kicker", kicker); grab("cta", cta ?? document.querySelector('button.btn-primary'));
    grab("greeting", greeting);
    grab("count", count); grab("dots", dots); grab("truth", truth); grab("size", size);
    out.tiles = tiles.length;
    out.step = document.querySelector('[data-step]')?.getAttribute('data-step') ?? null;
    out.windowW = window.innerWidth; out.windowH = window.innerHeight;
    out.panelRect = panel ? (() => { const r = panel.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; })() : null;
    return out;
  });
  out[step] = data;
  await context.close();
}

async function main() {
  if (!fs.existsSync(WEB_OUT)) throw new Error(`web export not found at ${WEB_OUT}; run npm --prefix app/web run build first`);
  const server = await serveStatic(CAPTURE_PORT, WEB_OUT);
  const base = `http://127.0.0.1:${CAPTURE_PORT}`;
  console.log(`probe: serving ${WEB_OUT} on ${base}`);
  const browser = await chromium.launch({ headless: true });
  const out = {};
  try {
    for (const step of ["hello", "case", "parts", "match", "finish"]) {
      await probe(browser, base, step, out);
      console.log(`probe: ${step} done`);
    }
  } finally {
    await browser.close();
    await new Promise((r) => server.close(r));
  }
  const file = path.resolve(root, ".build/ui-capture/212-probe.json");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(out, null, 2));
  console.log(`probe: wrote ${file}`);
}

await main();
