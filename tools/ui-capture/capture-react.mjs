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
//   UI_CAPTURE_PALETTE   comma-separated queries for the command palette
//       (#211): "empty" shoots the empty query at every size, any other
//       entry shoots that query at 1440x900 and 390x844. When set, only the
//       palette shots are taken (Layout page, Ctrl+K, query typed), as
//       palette[-q-<slug>]-react-<w>x<h>-<theme>.png.
//   UI_CAPTURE_FREEZE=1  freeze every thumbnail loop on frame 0, the frame
//       the GPUI editor always draws (its settle probe needs still frames).
//       Implemented as the browser's reduced-motion setting, which
//       `useMotionOff` already honours: `Thumb` and `EffectPreview` then
//       paint frame 0 and never advance, so layout can be diffed against
//       the GPUI frame without animated content in the way. Off by default:
//       the animated capture stays the record of how the page looks.
//   UI_CAPTURE_EDITOR=<effect name>  (#214) open the code editor on that
//       effect: the library's card is clicked and then "Edit code". The shot
//       is named editor-react-<w>x<h>-<theme>.png.
//   UI_CAPTURE_SCROLL=<px>  (#205) scroll the page down that far first; files
//       are <view>-scroll<px>-react-<w>x<h>-<theme>.png.
//   UI_CAPTURE_UPDATE=<preset>  (#206) the tray stub update state, a name from tools/ui-capture/update-presets.json
//       (0.2.0 with notes). The Updates shot is named updates-<preset>-react-...
//   UI_CAPTURE_TOAST=1  (#246) stage a Polling-rate change on the Peripherals
//       page and press Apply, so React's notify() toast is on screen; use with
//       UI_CAPTURE_VIEWS=peripherals and writes on for the mouse. The shot is
//       named peripherals-toast-react-<w>x<h>-<theme>.png.
//   UI_CAPTURE_STACK=<stack name>  (#214) open the stack builder on that
//       stack ("new" for a stack that has never been saved). The shot is
//       named stack-<slug>-react-<w>x<h>-<theme>.png.

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
const FREEZE = process.env.UI_CAPTURE_FREEZE === "1";
// #240: Chromium draws its text with ClearType (subpixel LCD) by default, while
// the GPUI editor draws with DirectWrite's grayscale antialiasing (gpui pins
// DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE). With UI_CAPTURE_GRAYSCALE=1 the
// browser draws the same grayscale, so a pair measures the LAYOUT and not the
// antialiasing. It is off by default: the committed pairs are the plain ones.
const GRAYSCALE = process.env.UI_CAPTURE_GRAYSCALE === "1";
const CHROMIUM_ARGS = GRAYSCALE ? ["--disable-lcd-text"] : [];
if (!/^http:\/\/(127\.0\.0\.1|localhost):\d+$/.test(DAEMON_BASE)) {
  throw new Error(`UI_CAPTURE_API_BASE must be http://127.0.0.1:<port> (the token is sent there), not ${DAEMON_BASE}`);
}

const VIEWS = [
  { name: "layout", path: "/" },
  { name: "effects", path: "/effects/" },
  { name: "devices", path: "/devices/" },
  { name: "fans", path: "/fans/" },
  { name: "settings", path: "/settings/" },
  { name: "updates", path: "/updates/" },
  { name: "displays", path: "/displays/" },
  { name: "peripherals", path: "/peripherals/" },
];

const VIEWPORTS = [
  { w: 1920, h: 1040 },
  { w: 1440, h: 900 },
  { w: 1280, h: 860 },
  { w: 1024, h: 768 },
  { w: 960, h: 1032 },
  { w: 390, h: 844 },
];

const THEMES = ["dark", "light"];

// Optional subsets for a focused run (comma lists); unset means the full
// matrix, exactly as before.
const VIEW_FILTER = (process.env.UI_CAPTURE_VIEWS ?? "")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
const VIEWPORT_FILTER = (process.env.UI_CAPTURE_VIEWPORTS ?? "")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
const THEME_FILTER = (process.env.UI_CAPTURE_THEMES ?? "")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
const ACTIVE_VIEWS = VIEW_FILTER.length ? VIEWS.filter((v) => VIEW_FILTER.includes(v.name)) : VIEWS;
const ACTIVE_VIEWPORTS = VIEWPORT_FILTER.length
  ? VIEWPORTS.filter((v) => VIEWPORT_FILTER.includes(`${v.w}x${v.h}`))
  : VIEWPORTS;
const ACTIVE_THEMES = THEME_FILTER.length ? THEMES.filter((t) => THEME_FILTER.includes(t)) : THEMES;
if (!ACTIVE_VIEWS.length || !ACTIVE_VIEWPORTS.length || !ACTIVE_THEMES.length) {
  throw new Error("a UI_CAPTURE_* filter matched nothing (check the names)");
}

// The command palette (#211): when UI_CAPTURE_PALETTE is set (a comma list of
// queries, "empty" for the empty query), only the palette shots are taken:
// the Layout page with Ctrl+K open, the query typed where one is given. The
// empty query shoots at every size; a typed query at 1440x900 and 390x844.
const PALETTE_ENTRIES = (process.env.UI_CAPTURE_PALETTE ?? "")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
const PALETTE_SIZES = (entries) =>
  entries === "empty" ? ACTIVE_VIEWPORTS : ACTIVE_VIEWPORTS.filter((v) => ["1440x900", "390x844", "1024x768"].includes(`${v.w}x${v.h}`));

// The effect editor (#214): the effect whose code is opened, by its name in
// the library. The shot is named editor-react-<w>x<h>-<theme>.png.
const EDITOR_EFFECT = (process.env.UI_CAPTURE_EDITOR ?? "").trim();
// The stack builder (#214): the stack that is opened, by name ("new" for a
// stack that has never been saved). The shot is named
// stack-<slug>-react-<w>x<h>-<theme>.png.
const STACK_NAME = (process.env.UI_CAPTURE_STACK ?? "").trim();
// The peripherals page (#208): the device selected before the shot
// ("mouse" | "controller"). The shot is named
// peripherals-<device>-react-<w>x<h>-<theme>.png.
const PERI_DEVICE = (process.env.UI_CAPTURE_DEVICE ?? "").trim();
// #246: UI_CAPTURE_TOAST=1 stages one Polling-rate change on the Peripherals
// page and presses Apply, so React's notify("The settings are on the device")
// is on screen when the shot is taken. Needs writes on for the device (the
// fake daemon starts with them off): PUT /api/v1/peripherals/<id>/access with
// {"reads":true,"writes":true}. The shot is named
// peripherals-toast-react-<w>x<h>-<theme>.png, pairing with
// tools/ui-capture/capture-gpui.ps1 -Toast.
const PERI_TOAST = process.env.UI_CAPTURE_TOAST === "1";
const PERI_DEVICE_REF = PERI_TOAST && PERI_DEVICE === "" ? "mouse" : PERI_DEVICE;
// #205: UI_CAPTURE_SCROLL=<px> scrolls the page that far down before the shot
// (Settings has panels below the fold at every size but the largest). The shot
// is named <view>-scroll<px>-react-<w>x<h>-<theme>.png.
const SCROLL_PX = Math.max(0, Number(process.env.UI_CAPTURE_SCROLL ?? 0) || 0);
// #206: UI_CAPTURE_UPDATE=<preset> sets the tray stub's update state (tools/
// ui-capture/capture-gpui.ps1 -UpdateState writes the same state on stdin).
const UPDATE_PRESET = (process.env.UI_CAPTURE_UPDATE ?? "").trim();
// The presets live in tools/ui-capture/update-presets.json, shared with
// capture-gpui.ps1 so both editors get the same state (#206).
const UPDATE_PRESETS = (() => {
  const file = JSON.parse(fs.readFileSync(path.resolve(__dirname, "update-presets.json"), "utf8"));
  const out = {};
  for (const [name, preset] of Object.entries(file.presets)) {
    const { withRelease, updatedWithRelease, ...fields } = preset;
    out[name] = {
      ...fields,
      ...(withRelease ? { available: file.release } : {}),
      ...(updatedWithRelease ? { updated_to: file.release } : {}),
    };
  }
  return out;
})();

function slugOf(text) {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

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
function tauriInit({ daemonBase, token, theme, updatePreset }) {
  window.__uiCaptureTheme = theme;
  try {
    localStorage.setItem("canvas-rgb.theme", theme);
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
    ...(updatePreset ?? {}),
  };
  const invoke = async (cmd, args) => {
    switch (cmd) {
      case "get_api_config":
        return { base_url: daemonBase, token };
      case "open_logs_folder":
        return undefined;
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

const STEPS = ["hello", "case", "parts", "match", "finish"];

// The text each screen's panel carries. The walk clicks the primary key and
// reads `data-step` back, but a headless page sometimes paints the header of
// the new screen over the body of the old one: the step attribute says
// "parts" while the panel still shows the Hello screen. The shot is checked
// against the text the screen must carry, so a half-painted page is retaken
// rather than saved.
const STEP_TEXT = {
  hello: "Map my PC",
  case: "Is this your case?",
  parts: "Put in what's in your PC.",
  match: "Which part is lit?",
  finish: "Watch your PC.",
};

// The onboarding takeover (#212): when UI_CAPTURE_ONBOARDING is set (a comma
// list of steps), only onboarding shots run in demo mode: no __TAURI__, so
// request() answers from demoRequest() with the ?state= token in
// UI_CAPTURE_STATE (default empty). Steps advance by clicking the primary
// button. Files: onboarding-<step>-react-<w>x<h>-<theme>.png.
const ONBOARD_STEPS = (process.env.UI_CAPTURE_ONBOARDING ?? "")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
const ONBOARD_STATE = (process.env.UI_CAPTURE_STATE ?? "").trim();

async function shootOnboarding(page, base, viewport, theme, step, daemonBase, token) {
  const fileName = `onboarding-${step}-react-${viewport.w}x${viewport.h}-${theme}.png`;
  const filePath = path.join(OUT_DIR, fileName);
  const demoMode = ONBOARD_STATE !== "";
  const q = demoMode ? `?state=${encodeURIComponent(ONBOARD_STATE)}` : "";
  if (demoMode) {
    // No __TAURI__: the demo backend answers (demo.ts request()); the theme
    // still lives in localStorage before load.
    await page.addInitScript((t) => {
      try { localStorage.setItem("canvas-rgb.theme", t); } catch {}
    }, theme);
  } else {
    // Happy path against the fake daemon (same state as GPUI).
    await page.addInitScript(tauriInit, { daemonBase, token, theme });
  }
  await page.goto(`${base}/${q}`, { waitUntil: "load", timeout: 30000 }).catch(() => {});
  // Wait for the shell's live marker.
  try {
    await page.waitForFunction(() => document.querySelector('[data-live="on"]') !== null, { timeout: 8000 });
  } catch {}
  // Advance to the requested step by clicking the primary button by label.
  const labels = ["Map my PC", "Looks right", "Match the lights", "Nothing's lit", "Continue", "Done", "Yes", "That's all of it"];
  const clickPrimary = async () => {
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
  // Walk forward until the requested step (Match→Finish needs every run skipped).
  for (let i = 0; i < 12; i++) {
    if ((await currentStep()) === step) break;
    if (!(await clickPrimary())) break;
  }
  await page.waitForTimeout(600);
  // The hello wave plays on the lights from the Hello screen onwards: it
  // takes two seconds to cross the PC and then holds a steady glow. The two
  // shots have to show the same light, and the wave's phase is a property
  // of when each capture is taken, so both wait for it to settle: the glow
  // is the same whatever moment the wave started at. The Finish screen
  // plays the sweep instead, and its shot lands on the band while it is
  // crossing, so that screen is not waited for.
  if (step !== "finish") {
    await page.waitForTimeout(4500);
  }
  // The pointer is left where the last click put it, so the key it clicked is
  // caught in its hover colour and the part under it carries a name tag. The
  // GPUI's capture never clicks anything, so the pairs would measure the
  // hover state against the resting state. Park the pointer off the key (the
  // top-left corner, over the header, where nothing is clickable) and let the
  // hover transition finish before the shot.
  await page.mouse.move(0, 0);
  await page.waitForTimeout(400);
  // The screen must be the one that was asked for: the step attribute says
  // so, and the panel's own words say so too. A half-painted page (the
  // header of the new screen over the body of the old one) is waited out.
  const want = STEP_TEXT[step];
  for (let i = 0; i < 20; i++) {
    const seen = await page.evaluate((t) => {
      const panel = document.querySelector('[data-step] section');
      return (panel?.textContent ?? "").includes(t);
    }, want);
    if (seen) break;
    if (i === 19) {
      throw new Error(`the onboarding screen "${step}" never showed its own text ("${want}")`);
    }
    await page.waitForTimeout(500);
  }
  await page.screenshot({ path: filePath });
  return { view: `onboarding-${step}`, path: "/", viewport: { width: viewport.w, height: viewport.h }, theme, file: fileName, pageErrors: [], consoleErrors: [], apiFailures: [], slowThumbnails: [], requestFailures: [], notLoaded: [] };
}

async function shootPage(page, base, view, viewport, theme, daemonBase, token, paletteQuery = null) {
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
  // The theme lives in localStorage before load (like the existing scripts)
  // and the __TAURI__ stand-in is injected on every navigation.
  await page.addInitScript(tauriInit, { daemonBase, token, theme, updatePreset: UPDATE_PRESETS[UPDATE_PRESET] ?? null });
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
  const viewFile = view.name === "updates" && UPDATE_PRESETS[UPDATE_PRESET] ? `updates-${UPDATE_PRESET}` : view.name;
  let fileName = `${viewFile}-react-${viewport.w}x${viewport.h}-${theme}.png`;
  if (PERI_DEVICE_REF && view.name === "peripherals") {
    // The peripherals page (#208): select the device before the shot, so the
    // pair shows the same device on both sides. The mouse is the first key
    // and selected by default; clicking it again just re-reads it.
    fileName = `peripherals-${PERI_DEVICE_REF}-react-${viewport.w}x${viewport.h}-${theme}.png`;
    const keyText = PERI_DEVICE_REF === "controller" ? "8BitDo Ultimate" : "Attack Shark";
    const key = page.locator(".rd-devkey", { hasText: keyText }).first();
    try {
      await key.waitFor({ timeout: 8000 });
      await key.click({ timeout: 8000 });
    } catch {
      notLoaded.push(`the ${PERI_DEVICE_REF} device key was not found`);
    }
    // Wait for the picked device's detail to land (head name follows the pick).
    const headText = PERI_DEVICE_REF === "controller" ? "8BitDo Ultimate" : "Attack Shark";
    try {
      await page.waitForFunction(
        (want) => (document.querySelector(".peripheral-head-name")?.textContent ?? "").includes(want),
        headText,
        { timeout: 10000 },
      );
    } catch {
      notLoaded.push(`the ${PERI_DEVICE_REF} detail did not land`);
    }
    await page.waitForTimeout(800);
  }
  if (PERI_TOAST && view.name === "peripherals" && PERI_DEVICE_REF === "mouse") {
    // #246: one change (any Polling rate the page is not showing), then
    // Apply. React's `notify` says "The settings are on the device"
    // (`PeripheralsView.tsx:371`) and this shot catches it before its 3 s
    // clock runs out.
    fileName = `peripherals-toast-react-${viewport.w}x${viewport.h}-${theme}.png`;
    const seg = page.locator('.seg[aria-label="Polling rate (Hz)"] .seg-item[aria-checked="false"]').first();
    try {
      await seg.click({ timeout: 8000 });
    } catch {
      notLoaded.push("the Polling rate segmented was not found or writes are off for the mouse");
    }
    const apply = page.getByRole("button", { name: "Apply" }).first();
    try {
      await apply.click({ timeout: 8000 });
    } catch {
      notLoaded.push("the Apply key was not found");
    }
    try {
      await page.waitForSelector(".notify-body", { timeout: 8000 });
    } catch {
      notLoaded.push("the notify toast never appeared");
    }
    await page.waitForTimeout(150);
  }
  if (EDITOR_EFFECT) {
    // The code editor (#214): the library's card for the effect is clicked,
    // then "Edit code" opens its source.
    fileName = `editor-react-${viewport.w}x${viewport.h}-${theme}.png`;
    const card = page.locator(`.effect-card-name:text-is("${EDITOR_EFFECT}")`).first();
    try {
      await card.click({ timeout: 8000 });
    } catch {
      notLoaded.push(`the effect card ${EDITOR_EFFECT} was not found`);
    }
    await page.waitForTimeout(1200);
    const edit = page.getByRole("button", { name: "Edit code" }).first();
    try {
      await edit.click({ timeout: 8000 });
    } catch {
      notLoaded.push("the Edit code key was not found");
    }
    // The preview is a render of the effect's own source: it can take a few
    // seconds, and the note reads "Rendering…" until it lands.
    await page.waitForTimeout(9000);
  }
  if (STACK_NAME) {
    // The stack builder (#214): the page is already on the stack's route.
    fileName = `stack-${slugOf(STACK_NAME)}-react-${viewport.w}x${viewport.h}-${theme}.png`;
    await page.waitForTimeout(1500);
  }
  if (paletteQuery !== null) {
    // The command palette (#211): Ctrl+K opens it, the query is typed, then
    // the shot. The slug names the file so the GPUI shot pairs with it.
    const slug = paletteQuery.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
    fileName = `palette${slug ? `-q-${slug}` : ""}-react-${viewport.w}x${viewport.h}-${theme}.png`;
    await page.keyboard.press("Control+k");
    try {
      await page.waitForSelector(".cmdk", { timeout: 5000 });
    } catch {
      notLoaded.push("command palette did not open after Ctrl+K");
    }
    // The input is focused 20 ms after mount: typing at once loses its
    // first character to the page.
    await page.waitForTimeout(200);
    if (paletteQuery) {
      await page.keyboard.type(paletteQuery, { delay: 10 });
    }
    await page.waitForTimeout(400);
  }
  if (SCROLL_PX > 0) {
    // #205: scroll the page by this many pixels before the shot, so the
    // panels below the fold can be compared. The Settings and Updates pages
    // scroll in their own pane from 721 px up and as a document below that;
    // the pane is used when it really scrolls, else the document. The file is
    // named <view>-scroll<px>-react-... and the GPUI shot pairs with it.
    fileName = `${viewFile}-scroll${SCROLL_PX}-react-${viewport.w}x${viewport.h}-${theme}.png`;
    const how = await page.evaluate((y) => {
      const pane = document.querySelector(".rd-settings-scroll, .rd-updates-grid, .rd-updates-notes");
      if (pane && pane.scrollHeight > pane.clientHeight + 1 && getComputedStyle(pane).overflowY !== "visible") {
        pane.scrollTop = y;
        return `pane ${pane.className}: scrollTop ${pane.scrollTop} of ${pane.scrollHeight - pane.clientHeight}`;
      }
      window.scrollTo(0, y);
      return `document: scrollY ${window.scrollY} of ${document.documentElement.scrollHeight - window.innerHeight}`;
    }, SCROLL_PX);
    process.stderr.write(`  scroll ${SCROLL_PX}: ${how}\n`);
    await page.waitForTimeout(300);
  }
  const filePath = path.join(OUT_DIR, fileName);
  // Park the pointer off any key, so a shot is the resting state and not a
  // hover state (the onboarding capture clicks through the flow).
  await page.mouse.move(0, 0);
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
  console.log(`ui-capture: daemon ${DAEMON_BASE} (${ACTIVE_VIEWS.length} views x ${ACTIVE_VIEWPORTS.length} viewports x ${ACTIVE_THEMES.length} themes)`);
  const browser = await chromium.launch({ headless: true, args: CHROMIUM_ARGS });
  const shots = [];
  try {
    if (ONBOARD_STEPS.length) {
      for (const step of ONBOARD_STEPS) {
        for (const viewport of ACTIVE_VIEWPORTS) {
          for (const theme of ACTIVE_THEMES) {
            const context = await browser.newContext({
              viewport: { width: viewport.w, height: viewport.h },
              deviceScaleFactor: 1,
              ...(FREEZE ? { reducedMotion: "reduce" } : {}),
            });
            const page = await context.newPage();
            process.stderr.write(`shoot onboarding-${step} ${viewport.w}x${viewport.h} ${theme} ...\n`);
            shots.push(await shootOnboarding(page, base, viewport, theme, step, DAEMON_BASE, token));
            await context.close();
          }
        }
      }
    } else if (PALETTE_ENTRIES.length) {
      const layout = VIEWS.find((v) => v.name === "layout");
      for (const entry of PALETTE_ENTRIES) {
        const query = entry === "empty" ? "" : entry;
        for (const viewport of PALETTE_SIZES(entry)) {
          for (const theme of ACTIVE_THEMES) {
            const context = await browser.newContext({
              viewport: { width: viewport.w, height: viewport.h },
              deviceScaleFactor: 1,
            });
            const page = await context.newPage();
            process.stderr.write(`shoot palette ${query ? JSON.stringify(query) : "(empty)"} ${viewport.w}x${viewport.h} ${theme} ...\n`);
            shots.push(await shootPage(page, base, layout, viewport, theme, DAEMON_BASE, token, query));
            await context.close();
          }
        }
      }
    } else {
      // The editor and the builder (#214) each have their own route: the
      // editor is the library's page with the code open, the builder is the
      // library's page with `?stack=`.
      const views = STACK_NAME
        ? [{ name: `stack-${slugOf(STACK_NAME)}`, path: `/effects/?stack=${encodeURIComponent(STACK_NAME)}` }]
        : EDITOR_EFFECT
          ? [{ name: "editor", path: "/effects/" }]
          : ACTIVE_VIEWS;
      for (const viewport of ACTIVE_VIEWPORTS) {
        for (const theme of ACTIVE_THEMES) {
          const context = await browser.newContext({
            viewport: { width: viewport.w, height: viewport.h },
            deviceScaleFactor: 1,
            // UI_CAPTURE_FREEZE=1: hold every thumbnail loop on frame 0.
            ...(FREEZE ? { reducedMotion: "reduce" } : {}),
          });
          const page = await context.newPage();
          for (const view of views) {
            process.stderr.write(`shoot ${view.name} ${viewport.w}x${viewport.h} ${theme} ...\n`);
            shots.push(await shootPage(page, base, view, viewport, theme, DAEMON_BASE, token));
          }
          await context.close();
        }
      }
    }
  } finally {
    await browser.close();
    await new Promise((r) => server.close(r));
  }
  const unexplainedConsole = (s) =>
    s.consoleErrors.filter(
      (m) => !(s.slowThumbnails.length > 0 && /status of 503 \(Service Unavailable\)/.test(m)),
    ).length;
  const failed = shots.filter(
    (s) =>
      s.pageErrors.length > 0 ||
      s.apiFailures.length > 0 ||
      s.requestFailures.length > 0 ||
      s.notLoaded.length > 0 ||
      unexplainedConsole(s) > 0,
  );
  const report = {
    generatedAt: new Date().toISOString(),
    base,
    daemonBase: DAEMON_BASE,
    capturePort: CAPTURE_PORT,
    views: ACTIVE_VIEWS.map((v) => v.name),
    viewports: ACTIVE_VIEWPORTS,
    themes: ACTIVE_THEMES,
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
