// Probe React's phone status strip, box by box, for every page (#244).
// Headless Chromium against the fake daemon (the same state the GPUI capture
// sees), so every number here is the number the diff is measured against.
//
//   node tools/ui-capture/probe-strip.mjs
//   UI_CAPTURE_VIEWPORTS=390x844 UI_CAPTURE_THEMES=dark node ...
//
// Writes .build/ui-capture/244-strip-probe.json.
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
const VIEWPORTS = (process.env.UI_CAPTURE_VIEWPORTS ?? "390x844")
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
const PAGES = (process.env.UI_CAPTURE_PAGES ?? "layout,effects,devices,displays,fans,settings,updates")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);

const ROUTES = {
  layout: "/",
  effects: "/effects/",
  devices: "/devices/",
  displays: "/displays/",
  fans: "/fans/",
  settings: "/settings/",
  updates: "/updates/",
  peripherals: "/peripherals/",
};

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

const MEASURE = `(() => {
  const out = { url: location.pathname };
  const pick = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return {
      sel,
      rect: { x: +r.x.toFixed(2), y: +r.y.toFixed(2), w: +r.width.toFixed(2), h: +r.height.toFixed(2) },
      right: +r.right.toFixed(2),
      display: cs.display,
      visibility: cs.visibility,
      overflow: cs.overflowX + "/" + cs.overflowY,
      text: (el.textContent || "").replace(/\\s+/g, " ").trim().slice(0, 80),
      font: { size: cs.fontSize, weight: cs.fontWeight, lineHeight: cs.lineHeight, family: cs.fontFamily.split(",")[0], letterSpacing: cs.letterSpacing },
      color: cs.color,
      bg: cs.backgroundColor,
      padding: cs.paddingTop + " " + cs.paddingRight + " " + cs.paddingBottom + " " + cs.paddingLeft,
      gap: cs.gap,
      flex: cs.flexGrow + " " + cs.flexShrink + " " + cs.flexBasis,
      minWidth: cs.minWidth,
      scrollW: el.scrollWidth,
      clientW: el.clientWidth,
      textOverflow: cs.textOverflow,
      whiteSpace: cs.whiteSpace,
    };
  };
  // Every direct child of the strip row, left to right: the row's own order.
  const inner = document.querySelector(".strip-inner");
  out.row = pick(".strip-inner");
  out.kids = inner ? Array.from(inner.children).map((c) => {
    const r = c.getBoundingClientRect();
    const cs = getComputedStyle(c);
    return {
      tag: c.tagName.toLowerCase(),
      cls: Array.from(c.classList).join(" "),
      rect: { x: +r.x.toFixed(2), y: +r.y.toFixed(2), w: +r.width.toFixed(2), h: +r.height.toFixed(2) },
      right: +r.right.toFixed(2),
      display: cs.display,
      text: (c.textContent || "").replace(/\\s+/g, " ").trim().slice(0, 80),
    };
  }) : [];
  out.brand = pick(".strip-brand");
  out.title = pick(".rd-title");
  out.titleH1 = pick(".rd-title h1");
  out.actions = pick(".rd-actions");
  out.now = pick(".strip-now");
  out.swatch = pick(".strip-swatch");
  out.nowText = pick(".strip-now-text");
  out.effectName = pick(".strip-effect-name");
  out.nowDetail = pick(".strip-now-detail");
  out.mark = pick(".strip-mark");
  out.markIcon = pick(".strip-mark-icon");
  out.markText = pick(".strip-mark-text");
  out.controls = pick(".strip-controls");
  out.powerBtn = pick(".strip-controls .btn");
  out.searchBtn = pick(".strip-controls .ibtn");
  // The row's own parts, so the budget can be read off the boxes.
  const kidsOf = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    return Array.from(el.children).map((c) => {
      const r = c.getBoundingClientRect();
      const cs = getComputedStyle(c);
      return {
        tag: c.tagName.toLowerCase(),
        cls: Array.from(c.classList).join(" "),
        rect: { x: +r.x.toFixed(2), y: +r.y.toFixed(2), w: +r.width.toFixed(2), h: +r.height.toFixed(2) },
        right: +r.right.toFixed(2),
        display: cs.display,
        gap: cs.gap,
        padding: cs.paddingTop + " " + cs.paddingRight + " " + cs.paddingBottom + " " + cs.paddingLeft,
        text: (c.textContent || "").replace(/\\s+/g, " ").trim().slice(0, 60),
      };
    });
  };
  out.controlsKids = kidsOf(".strip-controls");
  out.actionsKids = kidsOf(".rd-actions");
  // The actions' own subtree, so each key's box can be read off.
  const deep = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const walk = (e, depth) => {
      const r = e.getBoundingClientRect();
      const cs = getComputedStyle(e);
      const node = {
        depth,
        tag: e.tagName.toLowerCase(),
        cls: Array.from(e.classList).join(" "),
        rect: { x: +r.x.toFixed(2), y: +r.y.toFixed(2), w: +r.width.toFixed(2), h: +r.height.toFixed(2) },
        display: cs.display,
        gap: cs.gap,
        padding: cs.paddingTop + " " + cs.paddingRight + " " + cs.paddingBottom + " " + cs.paddingLeft,
        margin: cs.marginTop + " " + cs.marginRight + " " + cs.marginBottom + " " + cs.marginLeft,
        text: (e.textContent || "").replace(/\\s+/g, " ").trim().slice(0, 40),
      };
      const kids = Array.from(e.children).map((c) => walk(c, depth + 1));
      if (kids.length) node.kids = kids;
      return node;
    };
    return walk(el, 0);
  };
  out.actionsTree = deep(".rd-actions");
  out.controlsTree = deep(".strip-controls");
  // The widths of the now-playing name's candidate cut points, measured in
  // the browser with the name's own style, so the GPUI's ellipsis cut can
  // be compared against the same numbers.
  const nameEl = document.querySelector(".strip-effect-name");
  if (nameEl) {
    const cs = getComputedStyle(nameEl);
    const probe = document.createElement("span");
    probe.style.position = "absolute";
    probe.style.visibility = "hidden";
    probe.style.whiteSpace = "nowrap";
    probe.style.font = cs.font;
    probe.style.letterSpacing = cs.letterSpacing;
    probe.style.fontWeight = cs.fontWeight;
    probe.style.fontFamily = cs.fontFamily;
    probe.style.fontSize = cs.fontSize;
    document.body.appendChild(probe);
    const cands = [];
    for (const t of [
      "Nothing", "Nothing ", "Nothing i", "Nothing is", "Nothing is ", "Nothing is r",
      "Nothing is running", "Nothing…", "Nothing …", "Nothing i…", "Nothing is…",
    ]) {
      probe.textContent = t;
      cands.push({ text: t, w: +probe.getBoundingClientRect().width.toFixed(2) });
    }
    probe.remove();
    out.nameWidths = cands;
  }
  // The strip's height, and the page's own scroll offset (0).
  const strip = document.querySelector(".strip");
  if (strip) {
    const r = strip.getBoundingClientRect();
    out.stripRect = { x: +r.x.toFixed(2), y: +r.y.toFixed(2), w: +r.width.toFixed(2), h: +r.height.toFixed(2) };
  }
  out.bodyScrollW = document.body.scrollWidth;
  out.innerW = window.innerWidth;
  return out;
})()`;

async function probePage(browser, base, route, viewport, theme, token) {
  const context = await browser.newContext({ viewport: { width: viewport.w, height: viewport.h }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  await page.addInitScript(tauriInit, { daemonBase: DAEMON_BASE, token, theme });
  await page.goto(`${base}${route}`, { waitUntil: "load", timeout: 30000 }).catch(() => {});
  try { await page.waitForFunction(() => document.querySelector('[data-live="on"]') !== null, { timeout: 15000 }); } catch {}
  await page.waitForTimeout(2500);
  const data = await page.evaluate(MEASURE);
  await context.close();
  return data;
}

async function main() {
  if (!fs.existsSync(WEB_OUT)) throw new Error(`web export not found at ${WEB_OUT}; run npm --prefix app/web run build first`);
  const token = fs.existsSync(TOKEN_FILE) ? fs.readFileSync(TOKEN_FILE, "utf8").trim() : "";
  const server = await serveStatic(CAPTURE_PORT, WEB_OUT);
  const base = `http://127.0.0.1:${CAPTURE_PORT}`;
  console.log(`probe-strip: serving ${WEB_OUT} on ${base} (daemon ${DAEMON_BASE})`);
  const browser = await chromium.launch({ headless: true });
  const out = {};
  try {
    for (const theme of THEMES) {
      for (const viewport of VIEWPORTS) {
        for (const name of PAGES) {
          const route = ROUTES[name];
          if (!route) { console.log(`probe-strip: unknown page ${name}`); continue; }
          const key = `${name}-${viewport.w}x${viewport.h}-${theme}`;
          out[key] = await probePage(browser, base, route, viewport, theme, token);
          const n = out[key].effectName;
          const a = out[key].actions;
          console.log(
            `probe-strip: ${key}  name=${n ? `${n.rect.w}px "${n.text}"` : "none"}  ` +
            `swatch=${out[key].swatch ? out[key].swatch.rect.w : "-"}  ` +
            `title=${out[key].titleH1 ? out[key].titleH1.rect.w : "-"}  ` +
            `actions=${a ? `${a.rect.w} (display ${a.display})` : "none"}  ` +
            `mark=${out[key].mark ? out[key].mark.rect.w : "-"}  ` +
            `controls=${out[key].controls ? out[key].controls.rect.w : "-"}`
          );
        }
      }
    }
  } finally {
    await browser.close();
    await new Promise((r) => server.close(r));
  }
  const file = path.resolve(root, ".build/ui-capture/244-strip-probe.json");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(out, null, 2));
  console.log(`probe-strip: wrote ${file}`);
}

await main();
