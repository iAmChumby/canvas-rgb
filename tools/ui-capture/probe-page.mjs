// Probe React's real boxes for any page (#209, #244).
// Headless Chromium against the fake daemon (the same state the GPUI capture
// sees), so every number here is the number the diff is measured against.
//
// For each route x viewport x theme it dumps the whole page subtree: every
// element's rect, its computed font (size/weight/line-height), its box
// (padding/gap/border/radius/background) and its own text. CSS-module class
// names are hashed, so each class name is rewritten to its module name
// (app-module__title__1a2b3 -> app:title) before it is written out.
//
//   node tools/ui-capture/probe-page.mjs
//   UI_CAPTURE_ROUTE=/devices/ UI_CAPTURE_VIEWPORTS=390x844 UI_CAPTURE_THEMES=dark node ...
//
// Writes .build/ui-capture/209-probe.json (one file per route slug).
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(new URL("../poison-compare/js/package.json", import.meta.url));
const { chromium } = require("playwright");

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "../..");
// The fake daemon only allows CORS from the capture origin
// (`http://127.0.0.1:4310`, crates/canvas-daemon/src/fakes.rs), so the probe
// serves on the same port as capture-react.mjs.
const CAPTURE_PORT = Number(process.env.UI_CAPTURE_PORT ?? 4310);
const WEB_OUT = path.resolve(root, "app/web/out");
const DAEMON_BASE = (process.env.UI_CAPTURE_API_BASE ?? "http://127.0.0.1:19430").replace(/\/$/, "");
const TOKEN_FILE = process.env.UI_CAPTURE_TOKEN_FILE ?? path.resolve(root, ".build/fake-home/api-token");
const ROUTE = process.env.UI_CAPTURE_ROUTE ?? "/devices/";
const SLUG = process.env.UI_CAPTURE_SLUG ?? (ROUTE.replace(/[^a-z0-9]+/gi, "-").replace(/^-|-$/g, "") || "root");
const VIEWPORTS = (process.env.UI_CAPTURE_VIEWPORTS ?? "1440x900,390x844")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean)
  .map((s) => {
    const [w, h] = s.split("x");
    return { w: Number(w), h: Number(h) };
  });
const THEMES = (process.env.UI_CAPTURE_THEMES ?? "dark")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);

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

// The __TAURI__ stand-in: shaped exactly like app/web/src/lib/host.ts.
function tauriInit({ daemonBase, token, theme }) {
  window.__uiCaptureTheme = theme;
  try { localStorage.setItem("canvas-rgb.theme", theme); } catch {}
  const updateState = {
    current_version: "0.1.0",
    auto_check: true,
    last_check: null,
    phase: { kind: "up_to_date" },
    available: null,
    skipped_version: null,
    snoozed: false,
    updated_to: null,
  };
  const invoke = async (cmd) => {
    switch (cmd) {
      case "get_api_config": return { base_url: daemonBase, token };
      case "update_state": return { ...updateState };
      case "update_check_now": return { ...updateState };
      case "tray_prefs": return { editor: "react", channel: "stable" };
      default: return undefined;
    }
  };
  const listen = async () => () => {};
  window.__TAURI__ = { core: { invoke }, event: { listen } };
}

async function probe(browser, base, viewport, theme, token) {
  const context = await browser.newContext({ viewport: { width: viewport.w, height: viewport.h }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  if (process.env.UI_CAPTURE_DEBUG) {
    page.on("console", (m) => console.log(`  console ${m.type()}: ${m.text().slice(0, 300)}`));
    page.on("pageerror", (e) => console.log(`  pageerror: ${String(e).slice(0, 300)}`));
  }
  await page.addInitScript(tauriInit, { daemonBase: DAEMON_BASE, token, theme });
  await page.goto(`${base}${ROUTE}`, { waitUntil: "load", timeout: 30000 }).catch(() => {});
  try { await page.waitForFunction(() => document.querySelector('[data-live="on"]') !== null, { timeout: 15000 }); } catch {}
  await page.waitForTimeout(2500);
  const data = await page.evaluate(() => {
    const out = { windowW: window.innerWidth, windowH: window.innerHeight, nodes: [] };
    const friendly = (cls) =>
      (cls ?? "")
        .split(/\s+/)
        .map((c) => {
          const m = /^([a-z0-9]+)-module__([A-Za-z0-9_]+)__[A-Za-z0-9]+$/.exec(c);
          if (m) return `${m[1]}:${m[2]}`;
          return c;
        })
        .join(" ");
    const named = (sel) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: +r.x.toFixed(2), y: +r.y.toFixed(2), w: +r.width.toFixed(2), h: +r.height.toFixed(2) };
    };
    out.anchors = {
      app: named(".app"),
      main: named(".main"),
      content: named(".content"),
      page: named("[data-pane]"),
      list: named(".rd-dev-list"),
      strip: named(".strip"),
      stripInner: named(".strip-inner"),
    };
    // The page's own subtree, from the page root down.
    const root = document.querySelector("[data-pane]") ?? document.querySelector("main") ?? document.body;
    if (!root) return out;
    const walk = (el, depth) => {
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      const own = Array.from(el.childNodes)
        .filter((n) => n.nodeType === Node.TEXT_NODE)
        .map((n) => n.textContent)
        .join("")
        .replace(/\s+/g, " ")
        .trim();
      const kids = Array.from(el.children).filter((c) => {
        const cr = c.getBoundingClientRect();
        const ccs = getComputedStyle(c);
        return ccs.display !== "none" && !(cr.width === 0 && cr.height === 0);
      });
      out.nodes.push({
        depth,
        tag: el.tagName.toLowerCase(),
        cls: friendly(Array.from(el.classList).join(" ")),
        text: own.slice(0, 120),
        rect: { x: +r.x.toFixed(2), y: +r.y.toFixed(2), w: +r.width.toFixed(2), h: +r.height.toFixed(2) },
        font: { size: cs.fontSize, weight: cs.fontWeight, lineHeight: cs.lineHeight, family: cs.fontFamily.split(",")[0], letterSpacing: cs.letterSpacing },
        color: cs.color,
        bg: cs.backgroundColor,
        border: `${cs.borderTopWidth} ${cs.borderRightWidth} ${cs.borderBottomWidth} ${cs.borderLeftWidth}`,
        radius: cs.borderRadius,
        padding: `${cs.paddingTop} ${cs.paddingRight} ${cs.paddingBottom} ${cs.paddingLeft}`,
        margin: `${cs.marginTop} ${cs.marginRight} ${cs.marginBottom} ${cs.marginLeft}`,
        gap: cs.gap,
        display: cs.display,
        flexDirection: cs.flexDirection,
        flexWrap: cs.flexWrap,
        alignItems: cs.alignItems,
        justifyContent: cs.justifyContent,
        overflow: `${cs.overflowX}/${cs.overflowY}`,
        flex: `${cs.flexGrow} ${cs.flexShrink} ${cs.flexBasis}`,
        gridTemplateColumns: cs.gridTemplateColumns,
        gridTemplateRows: cs.gridTemplateRows,
        gridTemplateAreas: cs.gridTemplateAreas,
        order: cs.order,
        minWidth: cs.minWidth,
        width: cs.width,
        height: cs.height,
        position: cs.position,
        zIndex: cs.zIndex,
        textOverflow: cs.textOverflow,
      });
      for (const c of kids) walk(c, depth + 1);
    };
    walk(root, 0);
    // The whole document's text and icon boxes (strip and dock included), so a
    // pair's difference can be split into text rasterisation, icon
    // rasterisation and everything else (tools/ui-capture/masked-diff.py).
    const rr = (r) => [+r.x.toFixed(2), +r.y.toFixed(2), +r.width.toFixed(2), +r.height.toFixed(2)];
    out.textRects = [];
    const tw = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = tw.nextNode(); n; n = tw.nextNode()) {
      if (!n.nodeValue || !n.nodeValue.trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(n);
      for (const r of range.getClientRects()) if (r.width > 0 && r.height > 0) out.textRects.push(rr(r));
    }
    out.iconRects = [];
    for (const el of document.querySelectorAll("svg")) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) out.iconRects.push(rr(r));
    }
    return out;
  });
  await context.close();
  return data;
}

async function main() {
  if (!fs.existsSync(WEB_OUT)) throw new Error(`web export not found at ${WEB_OUT}; run npm --prefix app/web run build first`);
  const token = fs.existsSync(TOKEN_FILE) ? fs.readFileSync(TOKEN_FILE, "utf8").trim() : "";
  const server = await serveStatic(CAPTURE_PORT, WEB_OUT);
  const base = `http://127.0.0.1:${CAPTURE_PORT}`;
  console.log(`probe-page: serving ${WEB_OUT} on ${base} (daemon ${DAEMON_BASE}, route ${ROUTE})`);
  const browser = await chromium.launch({ headless: true });
  const out = {};
  try {
    for (const theme of THEMES) {
      for (const viewport of VIEWPORTS) {
        const key = `${viewport.w}x${viewport.h}-${theme}`;
        out[key] = await probe(browser, base, viewport, theme, token);
        const a = out[key].anchors;
        console.log(`probe-page: ${key} content=${a.content ? `${a.content.w}@${a.content.x}` : "-"} list=${a.list ? `${a.list.w}@${a.list.x}` : "-"} nodes=${out[key].nodes.length}`);
      }
    }
  } finally {
    await browser.close();
    await new Promise((r) => server.close(r));
  }
  const file = path.resolve(root, `.build/ui-capture/209-probe-${SLUG}.json`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(out, null, 2));
  console.log(`probe-page: wrote ${file}`);
}

await main();
