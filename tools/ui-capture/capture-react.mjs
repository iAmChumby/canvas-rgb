// Screenshots the real React editor against the fake daemon (#200).
//
// Serves `app/web/out` (the Next.js static export) on 127.0.0.1 and drives it
// with headless Chromium (Playwright, same engine as WebView2). The page gets
// a minimal `window.__TAURI__` stand-in shaped exactly like what
// `app/web/src/lib/host.ts` reads: `get_api_config` hands back the fake
// daemon's base URL and the test token from the test home, and the other tray
// commands the views call answer with harmless stubs. The editor then talks to
// the real fake daemon (`canvas-rgbd --fakes`, test port 19430).
//
// It never launches the tray and never starts a Next dev server. The token is
// only ever sent to 127.0.0.1:19430 (the page's fetch/WebSocket calls to the
// daemon); it is never logged.
//
// Usage:
//   (1) powershell -NoProfile -File tools/run-fake-daemon.ps1
//   (2) node tools/ui-capture/capture-react.mjs
//
// Env:
//   UI_CAPTURE_PORT      static server port (default 4310)
//   UI_CAPTURE_OUT       PNG + report.json dir (default .build/ui-capture)
//   UI_CAPTURE_API_BASE  daemon base URL (default http://127.0.0.1:19430)
//   UI_CAPTURE_TOKEN_FILE / FAKE_TOKEN_FILE  test token path
//       (default .build/fake-home/api-token)
//   UI_CAPTURE_FAULTS  JSON {"openrgb":"down","fan":"pawn_io_unavailable","dismissed":false}
//       #580: patches only the daemon answers the notes read, so the real fake daemon still
//       serves everything else and no real hardware or fan chip is touched. `fan` may be
//       pawn_io_unavailable or chip_not_found; `openrgb` may be down or connecting; `empty`
//       empties the device list; `dismissed` hides the page notes, not the strip.

import { createRequire } from "node:module";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Playwright comes from tools/poison-compare/js (no new npm dependency).
const require = createRequire(new URL("../poison-compare/js/package.json", import.meta.url));
const { chromium } = require("playwright");

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "../..");

const CAPTURE_PORT = Number(process.env.UI_CAPTURE_PORT ?? process.env.CANVAS_RGB_CAPTURE_PORT ?? 4310);
const OUT_DIR = path.resolve(root, process.env.UI_CAPTURE_OUT ?? ".build/ui-capture");
const WEB_OUT = path.resolve(root, "app/web/out");
const DAEMON_BASE = process.env.UI_CAPTURE_API_BASE ?? "http://127.0.0.1:19430";
if (!/^http:\/\/(127\.0\.0\.1|localhost):\d+$/.test(DAEMON_BASE)) {
  throw new Error(`UI_CAPTURE_API_BASE must be http://127.0.0.1:<port> (the token is sent there), not ${DAEMON_BASE}`);
}

const ALL_VIEWS = [
  { name: "scenes", path: "/" },
  { name: "effects", path: "/effects/" },
  { name: "devices", path: "/devices/" },
  { name: "fans", path: "/fans/" },
  { name: "settings", path: "/settings/" },
  { name: "updates", path: "/updates/" },
  { name: "displays", path: "/displays/" },
  { name: "peripherals", path: "/peripherals/" },
  // #577: Help & About shows the safe-start state and its mode action.
  { name: "help", path: "/help/" },
  { name: "credits", path: "/credits/" },
];

const ALL_VIEWPORTS = [
  { w: 1920, h: 1040 },
  { w: 1440, h: 900 },
  { w: 1280, h: 860 },
  { w: 1024, h: 768 },
  { w: 960, h: 1032 },
  { w: 390, h: 844 },
];

// UI_CAPTURE_VIEWS narrows or replaces the view list when only a page or two needs a run:
// a JSON array of { name, path } (#578), or comma-separated names from ALL_VIEWS (#577),
// e.g. UI_CAPTURE_VIEWS=scenes,help.
// UI_CAPTURE_VIEWPORTS does the same for sizes: a JSON array of { w, h }, or "1440x900,390x844".
function pick(raw, all, key) {
  if (!raw) return all;
  if (raw.trim().startsWith("[")) return JSON.parse(raw);
  const wanted = raw.split(",").map((s) => s.trim()).filter(Boolean);
  return all.filter((item) => wanted.includes(key(item)));
}
const VIEWS = pick(process.env.UI_CAPTURE_VIEWS, ALL_VIEWS, (v) => v.name);
const VIEWPORTS = pick(process.env.UI_CAPTURE_VIEWPORTS, ALL_VIEWPORTS, (v) => `${v.w}x${v.h}`);
const FAULTS = process.env.UI_CAPTURE_FAULTS ? JSON.parse(process.env.UI_CAPTURE_FAULTS) : null;

/** The #580 notes a screenshot needs, in the daemon's words, with stable fixture keys. */
function injectedFaults() {
  const out = [];
  if (FAULTS?.fan) {
    const chipNotFound = FAULTS.fan === "chip_not_found";
    out.push({
      kind: "fan_chip",
      key: chipNotFound ? "1111222233334444" : "aaaabbbbccccdddd",
      device: null,
      affected: "Fan headers can't be driven.",
      cause: chipNotFound
        ? "canvas-rgb found no supported fan chip on this board."
        : "The PawnIO driver, which canvas-rgb needs to reach the fan chip, is missing or did not load.",
      fix: chipNotFound
        ? "GPU fans can still be set from canvas-rgb."
        : "Install PawnIO, then restart canvas-rgb.",
      ignore_if: chipNotFound
        ? "Your motherboard has no NCT6687D chip; GPU fans are still controlled."
        : "Your motherboard sets the fans and you do not need canvas-rgb to.",
      dismissed: FAULTS.dismissed === true,
    });
  }
  if (FAULTS?.openrgb === "down") {
    out.push({
      kind: "open_rgb_down",
      key: "0123456789abcdef",
      device: null,
      affected: "Lights reached through OpenRGB are not available.",
      cause: "OpenRGB is not running, or its port is busy or refused.",
      fix: "Start OpenRGB.",
      ignore_if: "You have no lights that need OpenRGB.",
      dismissed: FAULTS.dismissed === true,
    });
  }
  return out;
}

/** Patches the status and fans answers only; every other request still comes from the fake daemon. */
async function injectFaults(page) {
  if (!FAULTS) return;
  const faults = injectedFaults();
  await page.route(/127\.0\.0\.1:\d+\/api\/v1\/(status|fans|devices)(?:\?|$)/, async (route) => {
    const answer = await route.fetch();
    const body = await answer.json();
    if (route.request().url().includes("/api/v1/status")) {
      body.openrgb = FAULTS.openrgb ?? body.openrgb;
      body.faults = faults;
      if (FAULTS.empty) {
        body.devices_connected = 0;
        body.devices_total = 0;
      }
    } else if (route.request().url().includes("/api/v1/devices") && FAULTS.empty) {
      body.devices = [];
    } else if (FAULTS.fan) {
      body.chip_problem = FAULTS.fan;
      body.chip_checked = true;
      body.chip = null;
      body.headers = [];
    }
    await route.fulfill({ response: answer, body: JSON.stringify(body) });
  });
}


const THEMES = ["dark", "light"];

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".svg": "image/svg+xml",
  ".webp": "image/webp",
  ".ico": "image/x-icon",
  ".txt": "text/plain; charset=utf-8",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
};

function tokenPath() {
  return (
    process.env.UI_CAPTURE_TOKEN_FILE ??
    process.env.FAKE_TOKEN_FILE ??
    path.resolve(root, ".build/fake-home/api-token")
  );
}

function readToken() {
  const file = tokenPath();
  if (!fs.existsSync(file)) {
    throw new Error(`test token not found at ${file}; start tools/run-fake-daemon.ps1 first`);
  }
  const token = fs.readFileSync(file, "utf8").trim();
  if (!token) throw new Error(`test token file is empty: ${file}`);
  return token;
}

// Thumbnail renders share two slots (the preview's). A page that asks for
// every effect's thumbnail at once gets 503s while two renders run, and the
// blank tile stays blank for the shot. Warm the daemon's thumbnail cache
// first, one change at a time, so the matrix then answers 200s. Wet Smear
// (a paint effect) is far slower than the 30 s render budget in a debug
// build and is given up for a while, so it is not warmed: the matrix records
// whatever the daemon answers for it. The token only ever goes to the daemon
// base (127.0.0.1:19430).
async function prewarmThumbnails(daemonBase, token) {
  const headers = { Authorization: `Bearer ${token}` };
  // The daemon may still be starting: wait for the API to answer.
  let effectsRes;
  for (let attempt = 0; attempt < 30; attempt++) {
    try {
      effectsRes = await fetch(`${daemonBase}/api/v1/effects`, { headers });
      if (effectsRes.ok) break;
    } catch {
      // not answering yet
    }
    if (attempt === 29) throw new Error("prewarm: the daemon did not answer the effects list");
    await new Promise((r) => setTimeout(r, 1000));
  }
  if (!effectsRes?.ok) throw new Error(`prewarm: effects list answered ${effectsRes?.status}`);
  const body = await effectsRes.json();
  const ids = (body.effects ?? [])
    .map((e) => e.id)
    .filter(Boolean)
    .filter((id) => !id.includes("wet-smear"));
  console.log(`ui-capture: prewarming ${ids.length} thumbnails ...`);
  for (const id of ids) {
    const url = `${daemonBase}/api/v1/effects/${encodeURIComponent(id)}/thumbnail`;
    // A slow render may take the renderer's budget; retry 503s a few times.
    let settled = false;
    for (let attempt = 0; attempt < 12; attempt++) {
      let res;
      try {
        res = await fetch(url, { headers });
      } catch {
        // The daemon can be briefly unreachable while a render runs; retry.
        await new Promise((r) => setTimeout(r, 5000));
        continue;
      }
      if (res.ok) {
        console.log(`ui-capture: prewarm ${id} ok`);
        settled = true;
        break;
      }
      if (res.status !== 503) throw new Error(`prewarm: thumbnail ${id} answered ${res.status}`);
      if (attempt === 11) {
        // Over the render budget even after retries (a debug build on a busy machine). The shots
        // list it under slowThumbnails, so the run goes on.
        console.log(`ui-capture: prewarm ${id} still 503 after retries; listed as a slow thumbnail`);
        settled = true;
        break;
      }
      await new Promise((r) => setTimeout(r, 2000));
    }
    if (!settled) throw new Error(`prewarm: the daemon never answered for thumbnail ${id}`);
  }
}

function serveStatic(port, dir) {
  const server = http.createServer((req, res) => {
    try {
      const url = new URL(req.url ?? "/", `http://127.0.0.1:${port}`);
      let pathname = decodeURIComponent(url.pathname);
      // Next.js static export with trailingSlash: "/effects/" -> effects/index.html
      let file = path.normalize(path.join(dir, pathname));
      if (!file.startsWith(dir)) {
        res.writeHead(403);
        res.end("forbidden");
        return;
      }
      if (fs.existsSync(file) && fs.statSync(file).isDirectory()) {
        file = path.join(file, "index.html");
      }
      // "/effects" without slash -> effects/index.html
      if (!fs.existsSync(file) && !path.extname(pathname)) {
        const alt = path.join(dir, pathname, "index.html");
        if (fs.existsSync(alt)) file = alt;
      }
      if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) {
        res.writeHead(404);
        res.end("not found");
        return;
      }
      const ext = path.extname(file).toLowerCase();
      res.writeHead(200, {
        "Content-Type": MIME[ext] ?? "application/octet-stream",
        "Cache-Control": "no-store",
      });
      fs.createReadStream(file).pipe(res);
    } catch (e) {
      res.writeHead(500);
      res.end(String(e));
    }
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve(server));
  });
}

// The __TAURI__ stand-in: shaped exactly like app/web/src/lib/host.ts
// (TauriGlobal { core.invoke, event.listen }). Commands from
// app/web/src/proto/tauri.ts that host.ts invokes.
function tauriInit({ daemonBase, token, theme, suppressChain, fault }) {
  window.__uiCaptureTheme = theme;
  window.__uiCaptureFault = fault ?? "";
  if (fault === "clipboard") {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: async () => { throw new Error("the clipboard refused the copy"); } },
    });
  }
  try {
    localStorage.setItem("canvas-rgb.theme", theme);
    if (suppressChain) {
      // A fresh fake home has an ARGB header with no chain record, so Scenes opens the
      // header-setup modal over the page. A focused capture of another surface says
      // "Later" for this launch, which is a session-storage id.
      const launch = "ui-capture-launch";
      sessionStorage.setItem("canvas-rgb.launch", launch);
      localStorage.setItem("canvas-rgb.chains-later", launch);
    }
  } catch {
    // storage can be blocked; the data-theme is also set below
  }
  let updateState = {
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
      case "open_logs_folder":
      case "open_service_logs_folder":
      case "reveal_diag_bundle":
        return undefined;
      // #575: the tray's diagnostics commands work with the daemon down. The stand-in answers
      // with fakes, so the Help page's list and saved states can both be captured.
      case "diag_bundle_list":
        if (window.__uiCaptureFault === "diag-error") {
          throw new Error("The diagnostics file list could not be read. Try again in a moment.");
        }
        if (window.__uiCaptureFault === "service-logs-included") {
          return {
            path: null,
            files: [
              { name: "config/zones.json", bytes: 1824 },
              { name: "service-logs/canvas-rgb-svc.2026-11-27.log", bytes: 2048 },
            ],
            replaced: 0,
            service_logs: null,
          };
        }
        return {
          path: null,
          files: [
            { name: "config/zones.json", bytes: 1824 },
            { name: "config/config.toml", bytes: 612 },
            { name: "logs/canvas-rgbd.2026-11-27.log", bytes: 8192 },
          ],
          replaced: 0,
          service_logs: "not included: the service log folder is not there",
        };
      case "diag_bundle_save":
        if (window.__uiCaptureFault === "service-logs-included") {
          return {
            path: "D:\\capture\\canvas-rgb-diag.zip",
            files: [{ name: "service-logs/canvas-rgb-svc.2026-11-27.log", bytes: 2048 }],
            replaced: 2,
            service_logs: null,
          };
        }
        return {
          path: "D:\\capture\\canvas-rgb-diag.zip",
          files: [{ name: "config/zones.json", bytes: 1824 }],
          replaced: 3,
          service_logs: "not included: the service log folder is not there",
        };
      case "report_prefill":
        return { version: "0.20.0", windows: "Windows 11 Pro 10.0.26100" };
      case "update_state":
        return { ...updateState };
      case "update_check_now":
        updateState = {
          ...updateState,
          last_check: new Date().toISOString(),
          phase: { kind: "up_to_date" },
        };
        return { ...updateState };
      case "update_install":
      case "update_later":
      case "update_skip":
      case "update_unskip":
      case "update_dismiss_updated":
        return { ...updateState };
      case "update_set_auto":
        updateState = { ...updateState, auto_check: args?.enabled === true };
        return { ...updateState };
      case "tray_prefs":
        return { editor: "react", channel: "stable" };
      case "set_editor":
      case "set_channel":
        return undefined;
      default:
        return undefined;
    }
  };
  const listen = async () => () => {};
  window.__TAURI__ = { core: { invoke }, event: { listen } };
}

async function shootPage(page, base, view, viewport, theme, daemonBase, token) {
  const pageErrors = [];
  const consoleErrors = [];
  const apiFailures = [];
  // A thumbnail that went over the daemon's render budget (a debug build renders slowly) answers
  // 503 and the editor shows a blank tile. Both editors see the same daemon, so it is listed, not failed.
  const slowThumbnails = [];
  const requestFailures = [];
  const notLoaded = [];
  const onRequestFailed = (req) => {
    if (req.url().startsWith(daemonBase)) requestFailures.push({ url: req.url(), error: req.failure()?.errorText ?? "failed" });
  };
  const onPageError = (err) => pageErrors.push(String(err?.stack ?? err));
  const onConsole = (msg) => {
    if (msg.type() === "error") consoleErrors.push(msg.text());
  };
  const onResponse = (res) => {
    const url = res.url();
    const status = res.status();
    if (status >= 400 && url.startsWith(daemonBase)) {
      const thumbnail = /\/api\/v1\/effects\/[^/]+\/thumbnail(\?|$)/.test(url.slice(daemonBase.length));
      if (status === 503 && thumbnail) slowThumbnails.push({ url, status });
      else apiFailures.push({ url, status });
    }
  };
  page.on("pageerror", onPageError);
  page.on("console", onConsole);
  page.on("response", onResponse);
  page.on("requestfailed", onRequestFailed);
  // UI_CAPTURE_FAULT drives the recovery surfaces without touching real services:
  //   down     - the daemon cannot be reached at all (DaemonDown)
  //   engine   - status answers 503, the render engine is down (EngineDown)
  //   service  - status reports the fan service away (the standing banner and its keys)
  //   inline   - one route answers a scrubbed error (the inline ErrorState anatomy)
  const fault = process.env.UI_CAPTURE_FAULT ?? "";
  if (fault === "down") {
    await page.route(`${daemonBase}/api/v1/status`, (route) => route.abort("failed"));
  } else if (fault) {
    await page.route(`${daemonBase}/api/v1/status`, async (route) => {
      if (fault === "engine" || fault === "long-message") {
        await route.fulfill({
          status: 503,
          contentType: "application/json",
          body: JSON.stringify({
            error: "service_unavailable",
            message: fault === "long-message"
              ? "the render engine stopped after a shader in the draft at line 42 could not be compiled; the daemon kept the last good frame and refused the next one, so the preview stayed dark and the lights held their last colours while the watcher waited for the renderer to answer again"
              : "the render engine is not answering",
            hint: "Save diagnostics and report the problem if it keeps happening.",
          }),
        });
        return;
      }
      const response = await route.fetch();
      if (fault !== "service") {
        await route.fulfill({ response });
        return;
      }
      const status = await response.json();
      status.service_down_ms = 14_000;
      await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(status) });
    });
    if (fault === "big-report") {
      await page.route(`${daemonBase}/api/v1/about/report`, (route) =>
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            markdown: "# canvas-rgb support report\n\n- version: 0.20.0",
            version: "0.20.0",
            windows: "Windows 11 Pro 10.0.26100",
            hardware: "Model-only line ".repeat(600),
            other_rgb_apps: "none seen",
          }),
        }),
      );
    }
    if (fault === "inline") {
      await page.route(`${daemonBase}/api/v1/devices`, (route) =>
        route.fulfill({
          status: 503,
          contentType: "application/json",
          body: JSON.stringify({
            error: "service_unavailable",
            message: "The fan service is not answering.",
            hint: "The fans stay on BIOS control. Try again in a moment.",
          }),
        }),
      );
    }
  }
  // The theme lives in localStorage before load (like the existing scripts)
  // and the __TAURI__ stand-in is injected on every navigation.
  await page.addInitScript(tauriInit, {
    daemonBase,
    token,
    theme,
    fault,
    suppressChain: Boolean(process.env.UI_CAPTURE_SUPPRESS_CHAIN_SETUP),
  });
  await injectFaults(page);
  const url = base + view.path;
  const loaded = await page.goto(url, { waitUntil: "load", timeout: 30000 });
  if (!loaded || !loaded.ok()) notLoaded.push(`page ${url} answered ${loaded ? loaded.status() : "nothing"}`);
  // The shell draws the page once the daemon has answered (data-live="on").
  // Wait for that; if the daemon is down the shot still records the screen.
  try {
    await page.waitForFunction(
      () => document.querySelector('[data-live="on"]') !== null,
      { timeout: 8000 },
    );
  } catch {
    // Recorded and counted as a failure: the view never showed the daemon's data.
    notLoaded.push(`no live data after 8 s (data-live="on" never appeared)`);
  }
  // Let the view settle (fonts, thumbnails, one preview frame).
  await page.waitForTimeout(400);
  // UI_CAPTURE_SCROLL_TO scrolls to a selector before the shot, for a section
  // that sits below the fold (#578).
  const scrollTo = process.env.UI_CAPTURE_SCROLL_TO;
  if (scrollTo) {
    await page.evaluate((selector) => {
      document.querySelector(selector)?.scrollIntoView({ block: "start" });
    }, scrollTo);
    await page.waitForTimeout(200);
  }
  // UI_CAPTURE_CLICK activates one selector before the shot (an opened
  // disclosure). It dispatches the click in the page rather than using
  // page.click: Playwright auto-scrolls to the element, which would push a
  // section captured with UI_CAPTURE_SCROLL_TO back out of view.
  const clickSelector = process.env.UI_CAPTURE_CLICK;
  if (clickSelector) {
    await page.$eval(clickSelector, (el) => el.click());
    await page.waitForTimeout(200);
  }
  const fileName = `${view.name}${scrollTo ? "-scrolled" : ""}${clickSelector ? "-open" : ""}-react-${viewport.w}x${viewport.h}-${theme}.png`;
  const filePath = path.join(OUT_DIR, fileName);
  await page.screenshot({ path: filePath });
  page.off("pageerror", onPageError);
  page.off("console", onConsole);
  page.off("response", onResponse);
  page.off("requestfailed", onRequestFailed);
  return {
    view: view.name,
    path: view.path,
    viewport: { width: viewport.w, height: viewport.h },
    theme,
    file: fileName,
    pageErrors,
    consoleErrors,
    apiFailures,
    slowThumbnails,
    requestFailures,
    notLoaded,
  };
}

async function main() {
  if (!fs.existsSync(WEB_OUT)) {
    throw new Error(`web export not found at ${WEB_OUT}; run npm --prefix app/web run build first`);
  }
  const token = readToken();
  await prewarmThumbnails(DAEMON_BASE, token);
  fs.mkdirSync(OUT_DIR, { recursive: true });
  const server = await serveStatic(CAPTURE_PORT, WEB_OUT);
  const base = `http://127.0.0.1:${CAPTURE_PORT}`;
  console.log(`ui-capture: serving ${WEB_OUT} on ${base}`);
  console.log(`ui-capture: daemon ${DAEMON_BASE} (${VIEWS.length} views x ${VIEWPORTS.length} viewports x ${THEMES.length} themes)`);
  const browser = await chromium.launch({ headless: true });
  const shots = [];
  try {
    for (const viewport of VIEWPORTS) {
      for (const theme of THEMES) {
        const context = await browser.newContext({
          viewport: { width: viewport.w, height: viewport.h },
          deviceScaleFactor: 1,
        });
        const page = await context.newPage();
        for (const view of VIEWS) {
          process.stderr.write(`shoot ${view.name} ${viewport.w}x${viewport.h} ${theme} ...\n`);
          shots.push(await shootPage(page, base, view, viewport, theme, DAEMON_BASE, token));
        }
        await context.close();
      }
    }
  } finally {
    await browser.close();
    await new Promise((r) => server.close(r));
  }
  const unexplainedConsole = (s) =>
    faultsOn ? 0 : s.consoleErrors.filter(
      (m) => !(s.slowThumbnails.length > 0 && /status of 503 \(Service Unavailable\)/.test(m)),
    ).length;
  // A deliberate fault is the surface under test: its 503 and its refused status request are
  // expected, so they are reported in the JSON but do not fail the run.
  const faultsOn = Boolean(process.env.UI_CAPTURE_FAULT);
  const failed = shots.filter(
    (s) =>
      s.pageErrors.length > 0 ||
      (!faultsOn && s.apiFailures.length > 0) ||
      (!faultsOn && s.requestFailures.length > 0) ||
      (!faultsOn && s.notLoaded.length > 0) ||
      unexplainedConsole(s) > 0,
  );
  const report = {
    generatedAt: new Date().toISOString(),
    base,
    daemonBase: DAEMON_BASE,
    capturePort: CAPTURE_PORT,
    views: VIEWS.map((v) => v.name),
    viewports: VIEWPORTS,
    themes: THEMES,
    shotCount: shots.length,
    failedShots: failed.length,
    pageErrors: shots.reduce((n, s) => n + s.pageErrors.length, 0),
    consoleErrors: shots.reduce((n, s) => n + s.consoleErrors.length, 0),
    apiFailures: shots.reduce((n, s) => n + s.apiFailures.length, 0),
    slowThumbnails: shots.reduce((n, s) => n + s.slowThumbnails.length, 0),
    requestFailures: shots.reduce((n, s) => n + s.requestFailures.length, 0),
    notLoaded: shots.reduce((n, s) => n + s.notLoaded.length, 0),
    unexplainedConsoleErrors: shots.reduce((n, s) => n + unexplainedConsole(s), 0),
    shots,
  };
  fs.writeFileSync(path.join(OUT_DIR, "report.json"), JSON.stringify(report, null, 2));
  console.log(
    `ui-capture: ${shots.length} shots, pageErrors=${report.pageErrors} consoleErrors=${report.consoleErrors} apiFailures=${report.apiFailures} slowThumbnails=${report.slowThumbnails} requestFailures=${report.requestFailures} notLoaded=${report.notLoaded} unexplainedConsoleErrors=${report.unexplainedConsoleErrors} failedShots=${failed.length}`,
  );
  console.log(`ui-capture: report ${path.join(OUT_DIR, "report.json")}`);
  if (failed.length > 0) process.exitCode = 1;
}

await main();
