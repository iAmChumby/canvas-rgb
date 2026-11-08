// Probe React's Peripherals boxes headlessly (fakes only, #208).
// Usage: UI_CAPTURE_API_BASE env, node tools/ui-capture/probe-peripherals.mjs
import { createRequire } from "node:module";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(new URL("../poison-compare/js/package.json", import.meta.url));
const { chromium } = require("playwright");

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "../..");
const WEB_OUT = path.resolve(root, "app/web/out");
const DAEMON_BASE = process.env.UI_CAPTURE_API_BASE ?? "http://127.0.0.1:19431";
const PORT = 4310;

const MIME = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".mjs": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8", ".png": "image/png", ".svg": "image/svg+xml", ".woff": "font/woff", ".woff2": "font/woff2", ".txt": "text/plain; charset=utf-8" };
function serveStatic(port, dir) {
  const server = http.createServer((req, res) => {
    const url = new URL(req.url ?? "/", `http://127.0.0.1:${port}`);
    let pathname = decodeURIComponent(url.pathname);
    let file = path.normalize(path.join(dir, pathname));
    if (!file.startsWith(dir)) { res.writeHead(403); res.end(); return; }
    if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, "index.html");
    if (!fs.existsSync(file) && !path.extname(pathname)) {
      const alt = path.join(dir, pathname, "index.html");
      if (fs.existsSync(alt)) file = alt;
    }
    if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { "Content-Type": MIME[path.extname(file).toLowerCase()] ?? "application/octet-stream", "Cache-Control": "no-store" });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((resolve) => server.listen(port, "127.0.0.1", () => resolve(server)));
}

function tauriInit({ daemonBase, token, theme }) {
  try { localStorage.setItem("canvas-rgb.theme", theme); } catch {}
  const invoke = async (cmd) => {
    if (cmd === "get_api_config") return { base_url: daemonBase, token };
    if (cmd === "update_state") return { current_version: "0.1.0", auto_check: true, last_check: null, phase: { kind: "up_to_date" }, available: null, skipped_version: null, snoozed: false, updated_to: null };
    if (cmd === "tray_prefs") return { editor: "react", channel: "stable" };
    return undefined;
  };
  window.__TAURI__ = { core: { invoke }, event: { listen: async () => () => {} } };
}

const token = fs.readFileSync(path.resolve(root, ".build/fake-home/api-token"), "utf8").trim();
const server = await serveStatic(PORT, WEB_OUT);
const browser = await chromium.launch({ headless: true });

async function measure(w, h, theme, device) {
  const context = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  await page.addInitScript(tauriInit, { daemonBase: DAEMON_BASE, token, theme });
  await page.goto(`http://127.0.0.1:${PORT}/peripherals/`, { waitUntil: "load", timeout: 30000 });
  await page.waitForFunction(() => document.querySelector('[data-live="on"]') !== null, { timeout: 8000 });
  const keyText = device === "controller" ? "8BitDo Ultimate" : "Attack Shark";
  await page.locator(".rd-devkey", { hasText: keyText }).first().click({ timeout: 8000 });
  await page.waitForFunction((want) => (document.querySelector(".peripheral-head-name")?.textContent ?? "").includes(want), keyText, { timeout: 10000 });
  await page.waitForTimeout(800);
  await page.screenshot({ path: path.resolve(root, `.build/probe-${w}x${h}-${device}.png`) });
  const out = await page.evaluate(() => {
    const box = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
    };
    const cs = (el, keys) => {
      if (!el) return null;
      const c = getComputedStyle(el);
      const o = {};
      for (const k of keys) o[k] = c[k];
      return o;
    };
    const res = {};
    res.viewport = { innerWidth: window.innerWidth, innerHeight: window.innerHeight, devicePixelRatio: window.devicePixelRatio };
    res.head = box(document.querySelector(".peripheral-head"));
    res.headText = box(document.querySelector(".peripheral-head-text"));
    res.headName = box(document.querySelector(".peripheral-head-name"));
    res.headSub = box(document.querySelector(".peripheral-head-sub"));
    res.headIcon = box(document.querySelector(".peripheral-head-icon"));
    // StateDot: the span with Connected text inside peripheral-head
    res.stateDot = box([...document.querySelectorAll(".peripheral-head *")].find((e) => e.textContent?.trim() === "Connected"));
    res.access = box(document.querySelector(".rd-peri-access"));
    res.panels = [...document.querySelectorAll(".rd-peri-sections > .panel, .stack > .panel, .peripheral-detail .panel")].map((p) => ({ title: p.querySelector(".panel-title, h2, h3")?.textContent ?? p.className, ...box(p) }));
    res.rows = [...document.querySelectorAll(".prop-item")].map((r) => {
      const label = r.querySelector(".field-label");
      const control = r.querySelector(".prop-control");
      const slider = r.querySelector(".slider");
      const num = r.querySelector(".num");
      const seg = r.querySelector(".seg");
      const btn = r.querySelector("button, .read-only");
      return { kind: r.dataset.kind, label: label?.textContent?.trim(), row: box(r), labelBox: box(label), controlBox: box(control), sliderBox: box(slider), numBox: box(num), segBox: box(seg), valueBox: box(btn) };
    });
    res.chips = [...document.querySelectorAll(".dpi-chip")].map((c) => box(c));
    res.sliderStyle = cs(document.querySelector(".slider"), ["flex", "width"]);
    res.segStyle = cs(document.querySelector(".seg"), ["height", "fontSize", "padding"]);
    res.chipStyle = cs(document.querySelector(".dpi-chip"), ["height", "fontSize", "padding"]);
    res.headTextStyle = cs(document.querySelector(".peripheral-head-text"), ["flex"]);
    res.labelStyle = cs(document.querySelector(".field-label"), ["fontSize", "lineHeight"]);
    return res;
  });
  await context.close();
  return out;
}

const results = {};
for (const [w, h] of [[1440, 900], [390, 844], [1920, 1040], [1024, 768]]) {
  for (const device of ["mouse", "controller"]) {
    results[`${device}-${w}x${h}-dark`] = await measure(w, h, "dark", device);
  }
}
await browser.close();
await new Promise((r) => server.close(r));
fs.writeFileSync(path.resolve(root, ".build/probe-peripherals.json"), JSON.stringify(results, null, 1));
console.log("probe written");
