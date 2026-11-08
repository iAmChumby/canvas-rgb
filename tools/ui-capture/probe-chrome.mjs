// Probe React's phone shell chrome: the bottom dock and the top strip, box by
// box (#240). Headless Chromium against the fake daemon (the same state the
// GPUI capture sees), so every number here is the number the diff is measured
// against.
//
//   node tools/ui-capture/probe-chrome.mjs
//   UI_CAPTURE_VIEWPORTS=390x844 UI_CAPTURE_THEMES=dark node ...
//
// Writes .build/ui-capture/240-chrome-probe.json.
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
const PAGES = (process.env.UI_CAPTURE_PAGES ?? "displays,devices,effects,layout,settings,updates,fans")
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
  const r2 = (n) => +n.toFixed(2);
  const rectOf = (el) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r2(r.x), y: r2(r.y), w: r2(r.width), h: r2(r.height), right: r2(r.right), bottom: r2(r.bottom) };
  };
  const fontOf = (cs) => ({
    size: cs.fontSize,
    weight: cs.fontWeight,
    lineHeight: cs.lineHeight,
    family: cs.fontFamily.split(",")[0],
    letterSpacing: cs.letterSpacing,
  });
  const pick = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const cs = getComputedStyle(el);
    return {
      sel,
      rect: rectOf(el),
      display: cs.display,
      position: cs.position,
      text: (el.textContent || "").replace(/\\s+/g, " ").trim().slice(0, 80),
      font: fontOf(cs),
      color: cs.color,
      bg: cs.backgroundColor,
      padding: cs.paddingTop + " " + cs.paddingRight + " " + cs.paddingBottom + " " + cs.paddingLeft,
      margin: cs.marginTop + " " + cs.marginRight + " " + cs.marginBottom + " " + cs.marginLeft,
      border: cs.borderTopWidth + " " + cs.borderTopStyle + " " + cs.borderTopColor,
      radius: cs.borderRadius,
      gap: cs.gap,
      flex: cs.flexGrow + " " + cs.flexShrink + " " + cs.flexBasis,
      justify: cs.justifyContent,
      alignItems: cs.alignItems,
      alignContent: cs.alignContent,
      gridTemplateRows: cs.gridTemplateRows,
      gridRow: cs.gridRow,
      overflow: cs.overflowX + "/" + cs.overflowY,
      textOverflow: cs.textOverflow,
      whiteSpace: cs.whiteSpace,
      transform: cs.transform,
      opacity: cs.opacity,
      zIndex: cs.zIndex,
      minWidth: cs.minWidth,
      flexDirection: cs.flexDirection,
    };
  };
  // The dock: the rail, which is a fixed bottom bar at 720 px and below.
  const nav = document.querySelector("nav.rail");
  out.dock = pick("nav.rail");
  out.dockRect = rectOf(nav);
  if (nav) {
    out.dockBorderTop = (() => {
      const cs = getComputedStyle(nav);
      return { width: cs.borderTopWidth, style: cs.borderTopStyle, color: cs.borderTopColor };
    })();
    // The bar's own padding box: where the links start.
    const cs = getComputedStyle(nav);
    out.dockContent = {
      padTop: cs.paddingTop, padRight: cs.paddingRight, padBottom: cs.paddingBottom, padLeft: cs.paddingLeft,
      borderTop: cs.borderTopWidth,
      contentTop: r2(nav.getBoundingClientRect().top + parseFloat(cs.borderTopWidth)),
    };
    // The active-page marker (the 3 px bar that slides).
    const marker = nav.querySelector(".rail-marker");
    out.dockMarker = pick(".rail-marker");
    out.dockMarkerRect = rectOf(marker);
    // Each link: its box, its icon, its name and the tile behind the active one.
    out.dockLinks = Array.from(nav.querySelectorAll(".rail-link")).map((el) => {
      const cs = getComputedStyle(el);
      const icon = el.querySelector(".rail-icon");
      const svg = icon ? icon.querySelector("svg") : null;
      const label = el.querySelector(".rail-label");
      const before = getComputedStyle(el, "::before");
      return {
        cls: Array.from(el.classList).join(" "),
        current: el.getAttribute("aria-current"),
        label: label ? (label.textContent || "").trim() : null,
        rect: rectOf(el),
        display: cs.display,
        padding: cs.paddingTop + " " + cs.paddingRight + " " + cs.paddingBottom + " " + cs.paddingLeft,
        margin: cs.marginTop + " " + cs.marginRight + " " + cs.marginBottom + " " + cs.marginLeft,
        flex: cs.flexGrow + " " + cs.flexShrink + " " + cs.flexBasis,
        height: cs.height,
        font: fontOf(cs),
        color: cs.color,
        bg: cs.backgroundColor,
        radius: cs.borderRadius,
        justifyItems: cs.justifyItems,
        alignContent: cs.alignContent,
        gridTemplateRows: cs.gridTemplateRows,
        rowGap: cs.rowGap,
        icon: icon ? {
          rect: rectOf(icon),
          display: getComputedStyle(icon).display,
          color: getComputedStyle(icon).color,
        } : null,
        svg: svg ? {
          rect: rectOf(svg),
          attrW: svg.getAttribute("width"),
          attrH: svg.getAttribute("height"),
          viewBox: svg.getAttribute("viewBox"),
          fill: svg.getAttribute("fill"),
          stroke: svg.getAttribute("stroke"),
          strokeWidth: svg.getAttribute("stroke-width"),
          weight: svg.getAttribute("weight"),
          pathD: (svg.querySelector("path") || {}).getAttribute
            ? svg.querySelector("path").getAttribute("d")
            : null,
        } : null,
        labelBox: label ? {
          rect: rectOf(label),
          font: fontOf(getComputedStyle(label)),
          color: getComputedStyle(label).color,
          lineHeight: getComputedStyle(label).lineHeight,
          clip: getComputedStyle(label).clip,
          position: getComputedStyle(label).position,
          overflow: getComputedStyle(label).overflow,
        } : null,
        before: {
          content: before.content,
          position: before.position,
          top: before.top, bottom: before.bottom, left: before.left, right: before.right,
          height: before.height,
          background: before.backgroundColor,
          radius: before.borderRadius,
          zIndex: before.zIndex,
        },
      };
    });
  }
  // The strip: the row and its parts.
  out.strip = pick(".strip");
  out.stripRect = rectOf(document.querySelector(".strip"));
  out.stripInner = pick(".strip-inner");
  out.stripBrand = pick(".strip-brand");
  out.stripBrandMark = pick(".strip-brand .brand-mark");
  out.stripBrandSvg = pick(".strip-brand svg");
  out.stripTitle = pick(".rd-title");
  out.stripTitleH1 = pick(".rd-title h1");
  out.stripActions = pick(".rd-actions");
  out.stripNow = pick(".strip-now");
  out.stripSwatch = pick(".strip-swatch");
  out.stripNowText = pick(".strip-now-text");
  out.stripEffectName = pick(".strip-effect-name");
  out.stripNowDetail = pick(".strip-now-detail");
  out.stripMark = pick(".strip-mark");
  out.stripMarkIcon = pick(".strip-mark-icon");
  out.stripMarkIconSvg = pick(".strip-mark-icon svg");
  out.stripMarkText = pick(".strip-mark-text");
  out.stripControls = pick(".strip-controls");
  out.stripPower = pick(".strip-controls .btn");
  out.stripPowerIcon = pick(".strip-controls .btn svg");
  out.stripSearch = pick(".strip-controls .ibtn");
  out.stripSearchIcon = pick(".strip-controls .ibtn svg");
  // Every element inside the strip row, in order, with its box: the row's own
  // parts, so the budget can be read off.
  const inner = document.querySelector(".strip-inner");
  out.stripRowKids = inner ? Array.from(inner.children).map((c) => ({
    cls: Array.from(c.classList).join(" "),
    rect: rectOf(c),
    display: getComputedStyle(c).display,
    text: (c.textContent || "").replace(/\\s+/g, " ").trim().slice(0, 40),
  })) : [];
  // The button's own box (React's .btn at size sm): padding, radius, font.
  out.stripButtons = Array.from(document.querySelectorAll(".strip-controls button")).map((el) => {
    const cs = getComputedStyle(el);
    const svg = el.querySelector("svg");
    return {
      cls: Array.from(el.classList).join(" "),
      aria: el.getAttribute("aria-label"),
      rect: rectOf(el),
      font: fontOf(cs),
      padding: cs.paddingTop + " " + cs.paddingRight + " " + cs.paddingBottom + " " + cs.paddingLeft,
      radius: cs.borderRadius,
      bg: cs.backgroundColor,
      color: cs.color,
      border: cs.borderTopWidth + " " + cs.borderTopColor,
      display: cs.display,
      gap: cs.gap,
      svg: svg ? { rect: rectOf(svg), attrW: svg.getAttribute("width"), attrH: svg.getAttribute("height") } : null,
    };
  });
  // The now-playing name's widths at the row's own style, so the GPUI's
  // ellipsis cut can be compared against the same numbers.
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
    for (const t of ["Nothing", "Nothing ", "Nothing i", "Nothing is", "Nothing is ", "Nothing is r", "Nothing is running", "Nothing…", "Nothing …", "Nothing i…", "Nothing is…"]) {
      probe.textContent = t;
      cands.push({ text: t, w: r2(probe.getBoundingClientRect().width) });
    }
    probe.remove();
    out.nameWidths = cands;
    // The canvas measureText of the same string: the glyph advances the
    // browser gives the shaper, next to the box it lays out.
    try {
      const cv = document.createElement("canvas");
      const ctx = cv.getContext("2d");
      ctx.font = cs.font;
      out.measureText = ["Nothing", "Nothing is running", "Layout", "Displays"].map((t) => ({
        text: t,
        width: r2(ctx.measureText(t).width),
        actualBoundingBoxLeft: r2(ctx.measureText(t).actualBoundingBoxLeft),
        actualBoundingBoxRight: r2(ctx.measureText(t).actualBoundingBoxRight),
        fontBoundingBoxAscent: r2(ctx.measureText(t).fontBoundingBoxAscent),
        fontBoundingBoxDescent: r2(ctx.measureText(t).fontBoundingBoxDescent),
      }));
      // One word at a time: the per-glyph advance of each letter.
      out.glyphAdvances = "Nothing is running".split(" ").map((w) => ({
        word: w,
        width: r2(ctx.measureText(w).width),
        boxWidth: r2((() => {
          probe.textContent = w;
          return probe.getBoundingClientRect().width;
        })()),
      }));
      out.wordWidths = ["Nothing", "is", "running", "Nothing is running", "Layout", "Displays", "Devices", "Effects", "Settings", "Updates", "Peripherals"].map((w) => ({
        word: w,
        measureText: r2(ctx.measureText(w).width),
        boxWidth: r2((() => { probe.textContent = w; return probe.getBoundingClientRect().width; })()),
      }));
    } catch (e) { out.measureTextError = String(e); }
  }
  out.bodyScrollW = document.body.scrollWidth;
  out.innerW = window.innerWidth;
  out.innerH = window.innerHeight;
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
  console.log(`probe-chrome: serving ${WEB_OUT} on ${base} (daemon ${DAEMON_BASE})`);
  const browser = await chromium.launch({ headless: true });
  const out = {};
  try {
    for (const theme of THEMES) {
      for (const viewport of VIEWPORTS) {
        for (const name of PAGES) {
          const route = ROUTES[name];
          if (!route) { console.log(`probe-chrome: unknown page ${name}`); continue; }
          const key = `${name}-${viewport.w}x${viewport.h}-${theme}`;
          out[key] = await probePage(browser, base, route, viewport, theme, token);
          const d = out[key].dockRect;
          const s = out[key].stripRect;
          console.log(
            `probe-chrome: ${key}  dock=${d ? `${d.x},${d.y} ${d.w}x${d.h}` : "-"}  ` +
            `strip=${s ? `${s.x},${s.y} ${s.w}x${s.h}` : "-"}  ` +
            `links=${out[key].dockLinks ? out[key].dockLinks.length : "-"}`
          );
        }
      }
    }
  } finally {
    await browser.close();
    await new Promise((r) => server.close(r));
  }
  const file = path.resolve(root, ".build/ui-capture/240-chrome-probe.json");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(out, null, 2));
  console.log(`probe-chrome: wrote ${file}`);
}

await main();
