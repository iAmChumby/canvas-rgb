// Probe React's stack builder in its two error states (#246).
//
// Headless Chromium against the fake daemon, the same state the GPUI capture
// sees. The probe drives the builder the way a person does: it adds a layer,
// presses Save (a new stack with no name asks for one), types a name in the
// naming dialog and presses Save again.
//
//   "naming": the name the rules refuse ("preview") keeps the dialog open with
//             the reason under the field (`#stack-name-error`).
//   "save":   the name the daemon refuses ("Fire", an effect's name) lets the
//             dialog close and the save fail, so the ErrorPanel is drawn above
//             the stage.
//
// For each state x viewport x theme it writes the shot
// .build/ui-capture/stack-<state>-error-react-<w>x<h>-<theme>.png (the GPUI
// shot of the same state pairs with it) and the measured boxes in
// .build/ui-capture/stack-errors-react.json.
//
//   node tools/ui-capture/probe-stack-errors.mjs
//   UI_CAPTURE_VIEWPORTS=1440x900 UI_CAPTURE_THEMES=dark node ...
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
const OUT_DIR = path.resolve(root, process.env.UI_CAPTURE_OUT ?? ".build/ui-capture");
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
// The name the daemon refuses (it is an effect's name) and the name the
// client's rule refuses (the preview route keeps it).
const NAMES = { naming: "preview", save: "Fire" };

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
  const invoke = async (cmd, args) => {
    switch (cmd) {
      case "get_api_config":
        return { base_url: daemonBase, token };
      case "tray_prefs":
        return { editor: "react", channel: "stable" };
      case "update_state":
      case "update_check_now":
      case "update_install":
      case "update_later":
      case "update_skip":
      case "update_unskip":
      case "update_dismiss_updated":
      case "update_set_auto":
        return { ...updateState };
      default:
        return undefined;
    }
  };
  const listen = async () => () => {};
  window.__TAURI__ = { core: { invoke }, event: { listen } };
}

function readToken() {
  if (!fs.existsSync(TOKEN_FILE)) throw new Error(`test token not found at ${TOKEN_FILE}`);
  const token = fs.readFileSync(TOKEN_FILE, "utf8").trim();
  if (!token) throw new Error(`test token file is empty: ${TOKEN_FILE}`);
  return token;
}

async function drive(page, base, state, theme) {
  await page.addInitScript(tauriInit, { daemonBase: DAEMON_BASE, token: readToken(), theme });
  await page.goto(`${base}/effects/?stack=new`, { waitUntil: "load", timeout: 30000 }).catch(() => {});
  await page.waitForFunction(() => document.querySelector('[data-live="on"]') !== null, { timeout: 20000 });
  await page.waitForTimeout(800);

  // A layer, so the Save key is live (React's Save is disabled with no layer).
  const add = page.getByRole("button", { name: "Add layer" }).first();
  await add.click({ timeout: 15000 });
  await page.waitForSelector(".rd-add-list .rd-add-row", { timeout: 15000 });
  await page.locator(".rd-add-list .rd-add-row").first().click({ timeout: 15000 });
  await page.waitForTimeout(1200);

  // Save: the builder asks for a name, because a new stack has none.
  await page.getByRole("button", { name: "Save", exact: true }).first().click({ timeout: 15000 });
  await page.waitForSelector(".rd-naming", { timeout: 15000 });
  await page.waitForTimeout(300);
  // The name, and the dialog's Save.
  await page.fill(".rd-naming input.name-field", NAMES[state]);
  await page.locator(".rd-picker-foot button", { hasText: "Save" }).first().click({ timeout: 15000 });

  if (state === "naming") {
    await page.waitForSelector("#stack-name-error", { timeout: 15000 });
  } else {
    // The dialog closed and the save failed: the ErrorPanel is above the stage.
    await page.waitForFunction(
      () => (document.querySelector(".rd-stack-main .error-line")?.textContent ?? "").includes("could not save"),
      { timeout: 20000 },
    );
  }
  await page.waitForTimeout(700);
}

async function shoot(browser, base, viewport, theme, state) {
  const context = await browser.newContext({ viewport: { width: viewport.w, height: viewport.h }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.setDefaultTimeout(20000);
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await drive(page, base, state, theme);
  const boxes = await page.evaluate((state) => {
    // Who has the focus, and what the name field wears for it (#246). React's
    // `NameField` takes the focus when it appears, so the field wears the
    // `:focus-visible` outline until something else takes it (a click on the
    // dialog's Save key does, the way a browser focuses a button).
    const focus = (() => {
      const el = document.querySelector(".rd-naming input.name-field");
      if (!el) return null;
      const active = document.activeElement;
      const cs = getComputedStyle(el);
      return {
        activeTag: active ? active.tagName : null,
        activeClass: active ? String(active.className ?? "") : null,
        activeText: active ? (active.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 40) : null,
        fieldIsActive: active === el,
        fieldFocusVisible: el.matches(":focus-visible"),
        outline: cs.outline,
        outlineOffset: cs.outlineOffset,
        outlineColor: cs.outlineColor,
        borderTopColor: cs.borderTopColor,
        value: el.value,
      };
    })();
    // The box, the words and the colour of an element, and of its box's parent.
    const probe = (selector) => {
      const el = document.querySelector(selector);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      const p = el.parentElement ? el.parentElement.getBoundingClientRect() : null;
      const pcs = el.parentElement ? getComputedStyle(el.parentElement) : null;
      return {
        selector,
        text: (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 200),
        rect: { x: +r.x.toFixed(2), y: +r.y.toFixed(2), w: +r.width.toFixed(2), h: +r.height.toFixed(2) },
        color: cs.color,
        fontSize: cs.fontSize,
        fontWeight: cs.fontWeight,
        fontFamily: cs.fontFamily.split(",")[0],
        lineHeight: cs.lineHeight,
        background: cs.backgroundColor,
        margin: `${cs.marginTop} ${cs.marginRight} ${cs.marginBottom} ${cs.marginLeft}`,
        padding: `${cs.paddingTop} ${cs.paddingRight} ${cs.paddingBottom} ${cs.paddingLeft}`,
        border: `${cs.borderTopWidth} ${cs.borderRightWidth} ${cs.borderBottomWidth} ${cs.borderLeftWidth}`,
        radius: cs.borderRadius,
        parent: p
          ? {
              rect: { x: +p.x.toFixed(2), y: +p.y.toFixed(2), w: +p.width.toFixed(2), h: +p.height.toFixed(2) },
              display: pcs.display,
              flexDirection: pcs.flexDirection,
              gap: pcs.gap,
              padding: `${pcs.paddingTop} ${pcs.paddingRight} ${pcs.paddingBottom} ${pcs.paddingLeft}`,
            }
          : null,
      };
    };
    const out = {
      naming: {
        focus,
        reason: probe("#stack-name-error"),
        field: probe(".rd-naming input.name-field"),
        card: probe(".rd-naming"),
        scrim: probe(".rd-scrim"),
        head: probe(".rd-picker-head"),
        foot: probe(".rd-picker-foot"),
      },
      save: {
        summary: probe(".rd-stack-main .error-line"),
        panel: probe(".rd-stack-main .panel"),
        title: probe(".rd-stack-main .panel-title"),
        details: probe(".rd-stack-main .load-detail > summary"),
        tryAgain: probe(".rd-stack-main .load-note-action button"),
        main: probe(".rd-stack-main"),
      },
    };
    return out[state];
  }, state);
  // Park the pointer off any key, so the shot is the resting state.
  await page.mouse.move(0, 0);
  await page.waitForTimeout(300);
  const file = `stack-${state}-error-react-${viewport.w}x${viewport.h}-${theme}.png`;
  await page.screenshot({ path: path.join(OUT_DIR, file) });
  await context.close();
  return { state, viewport, theme, file, boxes, pageErrors: errors };
}

async function main() {
  if (!fs.existsSync(WEB_OUT)) throw new Error(`web export not found at ${WEB_OUT}`);
  fs.mkdirSync(OUT_DIR, { recursive: true });
  const server = await serveStatic(CAPTURE_PORT, WEB_OUT);
  const base = `http://127.0.0.1:${CAPTURE_PORT}`;
  const browser = await chromium.launch({ headless: true });
  const shots = [];
  try {
    for (const viewport of VIEWPORTS) {
      for (const theme of THEMES) {
        for (const state of ["naming", "save"]) {
          process.stderr.write(`shoot stack-${state}-error ${viewport.w}x${viewport.h} ${theme} ...\n`);
          shots.push(await shoot(browser, base, viewport, theme, state));
        }
      }
    }
  } finally {
    await browser.close();
    await new Promise((r) => server.close(r));
  }
  fs.writeFileSync(path.join(OUT_DIR, "stack-errors-react.json"), JSON.stringify(shots, null, 2));
  for (const shot of shots) {
    console.log(`${shot.file}: ${JSON.stringify(shot.boxes)}`);
  }
  console.log(`probe-stack-errors: ${shots.length} shots, boxes in ${path.join(OUT_DIR, "stack-errors-react.json")}`);
}

await main();
