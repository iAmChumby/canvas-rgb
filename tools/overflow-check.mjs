// Controls that run past their panel or the window, measured in a real browser (issues #263, #268, #285).
//
// jsdom has no layout, so a unit test cannot see a select that is wider than the Lighting panel
// or a "Reset to automatic" key that sticks out of its note. This serves the static export
// (app/web/out) on loopback, opens every row of VIEWS below (every view of the demo, and the states
// of them that need a click or a flag) at the six redesign sizes in both colour schemes, and
// reports, with the element named:
//   - an element inside a box (a `.panel`, a `.rule` note, or the row containers a view lists in
//     VIEWS) whose right edge passes that box's right edge by more than 1 px;
//   - a box (a `.rule`, or a panel inside a panel) whose right edge passes the right edge of the
//     panel that holds it;
//   - any element, inside a panel or not, whose right or left edge passes the window, and any
//     `.panel` wider than the window. `.content { overflow: hidden }` and every other clip hide
//     such an element; they do not excuse it (#285);
//   - a view whose document scrolls sideways (scrollWidth > clientWidth);
//   - a panel that scrolls sideways itself (its content is wider than it shows);
//   - a figure's unit that has wrapped under the figure ("70" over "°C", #268);
//   - a portal (a Popover or a Select's list, both drawn into `body`) that passes the window on any
//     side, or the right edge of the box its opening control sits in;
//   - on a desktop window (721 px and over, where a view fits the window and the document never
//     scrolls: R1), a view's own scroll pane (FIT_PANES) that scrolls vertically, and each panel
//     it cuts off at the bottom (#294). A pane is where R1 sends what is taller than the window; a
//     desktop size is meant to have nothing taller. A phone scrolls the document, not a pane.
// The slack is 1 px. Left out of every check: an element not drawn (display none, visibility hidden,
// opacity 0: a tooltip before hover; a fixed sheet or toast that is parked that way counts too), an
// element that is only there for a screen reader (1-2 px), and an element inside a scroller that
// scrolls sideways on purpose (its CSS says `overflow-x: auto|scroll`) whose own right edge is within
// the edge measured. A scroller is not a clip, and `overflow-y: auto` is not a sideways scroller: a
// list that scrolls vertically computes `overflow-x: auto` too, and what it holds past the panel is
// a bug, not a scroll. A `.panel` is never exempt (as the Fans right panel shows). Left out of the
// box checks only: an element fixed to the window (a toast, a dialog); the window check measures it,
// off-screen or not, so a shown one that passes the window is reported. Left out of the window check
// only (KNOWN_CLIPPED): Layout's `canvas.stage-canvas` past the window's right edge, while its own
// `.stage` clips it and sits inside the window (the zoomed view, clipped by its stage on purpose).
// A portal whose opening control cannot be found is reported as UNKNOWN, not as a pass.
//
// Only the topmost offender in a chain is reported: when a row overflows, its children do too,
// and listing each would bury the row.
//
// A view with nothing measured FAILS. The one exception is the list NO_PANEL below, the views
// that are known to draw no `.panel`; they list their own row containers in `boxes`, and a view in
// the list that draws none of those fails as well.
//
// What it measures is VIEWS and nothing else: the first paint of each view, plus the rows that
// open a state by a flag (`?state=`, `?stack=`) or by clicks (Fans with a limit set, with a limit
// inside the CPU's idle range, and in Curve mode; the effect editor and the StackBuilder; Layout
// with a zone and with a part selected and
// with the Add part popover open; Fans with the temperature source list open). The popup rows have
// `overlay: true`: they are skipped by `--hover-tips`, where an open list would cover what is
// hovered. A state that is not a row (a dialog, a toast, the onboarding, a stack's other layers,
// each of the other popovers) is not measured. Nothing waits a fixed time: a row is measured once
// the fonts are ready, its anchor is drawn, no load frame is still loading and every animation
// that ends has ended.
//
// Usage (after `npm --prefix app/web run build`; needs Playwright in tools/poison-compare/js, and
// a Chromium that Playwright can start, or Chrome/Edge installed):
//   node tools/overflow-check.mjs                        exit 1 on any hit
//   node tools/overflow-check.mjs --sizes 1440x900       fewer sizes
//   node tools/overflow-check.mjs --views fans,peripherals
//   node tools/overflow-check.mjs --shots <dir> <tag>    save <dir>/<tag>-<view>-<WxH>-<scheme>.png of every row
//   node tools/overflow-check.mjs --out <dir>            serve another export than app/web/out
//   node tools/overflow-check.mjs --mutate-overflow      plant, in every row, one overflow per
//                                                        measuring path (the modes of PLANT:
//                                                        box, outside, outside-left, panel, wide,
//                                                        doc, scroll-list, scrolls, wrapped,
//                                                        portal, canvas-left, canvas-unclipped,
//                                                        stage-sibling, pane) and FAIL when a row does
//                                                        not report one. `--mutate-overflow
//                                                        box,doc` plants only those.
//   node tools/overflow-check.mjs --hover-tips           hover every control that draws a tooltip
//                                                        and FAIL when a shown bubble passes the
//                                                        window by more than 1 px, or is cut off
//                                                        by an ancestor that clips it (#271). It
//                                                        replaces the edge measure above for the
//                                                        run; combine with --views and --sizes.

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "..");
const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  return i === -1 ? null : args[i + 1];
};
const out = path.resolve(flag("--out") ?? path.join(repo, "app", "web", "out"));
// Playwright lives in tools/poison-compare/js (as for tools/nav-check.mjs). A worktree that has not
// run `npm ci` there can point PLAYWRIGHT_DIR at another checkout's copy of that folder.
const pwDir = [process.env.PLAYWRIGHT_DIR && path.resolve(process.env.PLAYWRIGHT_DIR), path.join(repo, "tools", "poison-compare", "js")]
  .filter(Boolean)
  .find((d) => fs.existsSync(path.join(d, "node_modules", "playwright")));
if (!pwDir) throw new Error("Playwright not found: run `npm ci` in tools/poison-compare/js, or set PLAYWRIGHT_DIR");
const require = createRequire(path.join(pwDir, "package.json"));
const { chromium } = require("playwright");

const shotsDir = flag("--shots");
const shotTag = shotsDir ? args[args.indexOf("--shots") + 2] : null;
const MODES = [
  "box", "outside", "outside-left", "panel", "wide", "doc", "scroll-list", "scrolls", "wrapped", "portal",
  "canvas-left", "canvas-unclipped", "stage-sibling", "pane",
];
const mutateAt = args.indexOf("--mutate-overflow");
const mutate = mutateAt !== -1;
const modeArg = args[mutateAt + 1];
const mutateModes = !mutate ? [] : modeArg && !modeArg.startsWith("--") ? modeArg.split(",") : MODES;
for (const m of mutateModes) if (!MODES.includes(m)) throw new Error(`unknown --mutate-overflow mode "${m}" (${MODES.join(", ")})`);
const hoverTips = args.includes("--hover-tips");
const SIZES = (flag("--sizes") ?? "1920x1040,1440x900,1280x860,1024x768,960x1032,390x844")
  .split(",")
  .map((s) => s.split("x").map(Number));
const SCHEMES = ["dark", "light"];
/**
 * The scroll panes R1 gives a view for a window that is too short for it (see the "R1 on a desktop
 * window" block of app/web/src/styles/app.css). On a desktop size (a window over PHONE_MAX wide)
 * none of them is meant to scroll: the fit-window rule is that every panel fits, so a pane that has
 * something to scroll for is a view that does not fit (#294). Settings' `.rd-settings-scroll`, the
 * Peripherals detail, the Effects inspector and the Fans right panel. What's new's notes and its
 * one-column grid are left out: they scroll by design (#124), since release notes have no fixed
 * length. The mutation plants its own pane under the first of these classes.
 */
const FIT_PANES = ".rd-settings-scroll, .peripheral-detail, .rd-eff-inspector, .rd-fans-right > .panel";
const SLACK_PX = 1;
const T = 20000;
const PHONE_MAX = 720;

const hasCards = (page) => page.locator(".fan-cards").count().then((n) => n > 0);

/** Fans: pick the CPU fan, then set a "Don't exceed" limit so Reset to automatic is drawn. */
async function fansWithLimit(page) {
  await page.waitForSelector('[aria-label="Select CPU fan"], .fan-cards button', { timeout: T });
  if (await hasCards(page)) {
    await page.locator(".fan-cards button", { hasText: "CPU fan" }).first().click();
  }
  const f = page.getByLabel("Don't exceed temperature", { exact: true });
  await f.fill("70");
  await f.press("Enter");
  // The key is drawn (the hidden copy that holds its room is aria-hidden), and it is no longer
  // sending the limit.
  await page.waitForSelector('button:not([aria-hidden="true"]):has-text("Reset to automatic"):not([aria-busy="true"])', { timeout: T });
}

/**
 * Fans: a "Don't exceed" line inside the CPU's idle range (issue #297), so the "the fans will
 * run hard to hold it" warning is drawn under the control.
 */
async function fansIdleLimit(page) {
  await fansWithLimit(page);
  const f = page.getByLabel("Don't exceed temperature", { exact: true });
  await f.click();
  await f.press("Control+a");
  await f.pressSequentially("50", { delay: 20 });
  await f.press("Enter");
  await page.waitForSelector(".hot-warn", { timeout: T });
}

/** Fans: select the CPU fan (a card on a phone), the one the later rows act on. */
async function fansCpu(page) {
  await page.waitForSelector('[aria-label="Select CPU fan"], .fan-cards button', { timeout: T });
  if (await hasCards(page)) {
    await page.locator(".fan-cards button", { hasText: "CPU fan" }).first().click();
  }
}

/** Fans: the CPU fan in Curve mode, its curve editor drawn. */
async function fansCurve(page) {
  await fansCpu(page);
  const curve = page.getByRole("radio", { name: "Curve", exact: true });
  await curve.waitFor({ timeout: T });
  if ((await curve.getAttribute("aria-checked")) !== "true") await curve.click();
  await page.waitForSelector(".curve-rows", { timeout: T });
}

/** Effects: the first user effect (Sample HTML, from `?state=html`), then its code editor. */
async function effectEditor(page) {
  await page.getByRole("button", { name: "Open Sample HTML", exact: true }).click();
  await page.getByRole("button", { name: "Edit code", exact: true }).click();
  await page.waitForSelector(".eff-editor", { timeout: T });
}

/**
 * Fans: the CPU fan's temperature source list (a Select's listbox, drawn into body) open, in Curve
 * mode. (Peripherals' LED mode select is disabled until the demo allows writes, so it cannot open.)
 */
async function sourceList(page) {
  await fansCurve(page);
  await page.getByRole("combobox", { name: "CPU fan temperature source", exact: true }).click();
  await page.waitForSelector(".listbox", { timeout: T });
}

/** Settings: a microphone picked in Audio source (a Select, its list drawn into body), the Listen toggle drawn. */
async function settingsMic(page) {
  await page.getByRole("combobox", { name: "Audio source", exact: true }).click();
  await page.waitForSelector(".listbox", { timeout: T });
  await page.getByRole("option", { name: "Microphone", exact: true }).click();
  await page.getByRole("switch", { name: "Listen to this input", exact: true }).waitFor({ timeout: T });
}

/** Settings: the microphone picked and Listen on, so the state line carries a live reading's words. */
async function settingsListen(page) {
  await settingsMic(page);
  await page.getByRole("switch", { name: "Listen to this input", exact: true }).click();
  await page.waitForSelector('[role="status"]:not([data-note="true"])', { timeout: T });
}

/**
 * Every view of the demo, and the states of it that draw more than the first paint does.
 * `boxes` adds row containers to measure besides `.panel` and `.rule`. `prep` clicks the row into
 * its state (`before` is what to wait for first, when that is not a panel). `overlay` marks a row with a popup open: `--hover-tips` skips it.
 */
/** The Scenes page draws rows, not panels: the rows are its boxes. */
const SCENE_BOXES = "[data-testid=group-row], [data-testid=unassigned-row]";

const VIEWS = [
  // Scenes: the default (four groups and an unassigned light, one group with a device away), and
  // the states that change a row's height or width: a failed group, 40-character names, twenty
  // groups (the list scrolls), the notice for a new light, one group, no group.
  { name: "scenes", url: "/", boxes: SCENE_BOXES },
  { name: "scenes-failed", url: "/?state=scene:evening", boxes: SCENE_BOXES },
  { name: "scenes-long", url: "/?state=long-names", boxes: SCENE_BOXES },
  { name: "scenes-many", url: "/?state=groups-20", boxes: SCENE_BOXES },
  { name: "scenes-new", url: "/?state=new-light", boxes: SCENE_BOXES },
  { name: "scenes-one", url: "/?state=scene:focus", boxes: SCENE_BOXES },
  { name: "scenes-empty", url: "/?state=groups-0", boxes: SCENE_BOXES },
  { name: "effects", url: "/effects/", boxes: ".effect-toolbar, .effect-card" },
  { name: "effects-editor", url: "/effects/?state=html", before: ".effect-card", prep: effectEditor },
  { name: "effects-stack", url: "/effects/?stack=Sunset%20glow&state=complete,stack" },
  { name: "devices", url: "/devices/", boxes: ".rd-dev" },
  { name: "displays", url: "/displays/" },
  { name: "peripherals", url: "/peripherals/" },
  { name: "peripherals-read", url: "/peripherals/?state=reads-on" },
  { name: "fans", url: "/fans/" },
  { name: "fans-limit", url: "/fans/", prep: fansWithLimit },
  { name: "fans-idle-limit", url: "/fans/", prep: fansIdleLimit },
  { name: "fans-curve", url: "/fans/", prep: fansCurve },
  { name: "fans-source", url: "/fans/", prep: sourceList, overlay: true },
  { name: "settings", url: "/settings/" },
  // The states that make a Settings panel taller: a microphone picked (the Listen toggle, the
  // effect note), Listen on, a release waiting, an experimental build (the Editor choice), and
  // an install that failed with a long message (the update status line wraps).
  { name: "settings-mic", url: "/settings/", prep: settingsMic },
  { name: "settings-listen", url: "/settings/", prep: settingsListen },
  { name: "settings-update", url: "/settings/?update=available" },
  { name: "settings-dev", url: "/settings/?update=dev" },
  { name: "settings-failed", url: "/settings/?update=install-failed-text" },
  // No tray in a plain browser: the page waits, then says so (one panel). `?update=available` is
  // the fixture with a release waiting: its update card and the notes' panels.
  { name: "updates", url: "/updates/" },
  { name: "updates-available", url: "/updates/?update=available" },
];
/**
 * The views known to draw no `.panel` (Devices is a list of rows; Effects on a phone is the
 * gallery). They are measured through their `boxes` instead, and the check fails if those are
 * missing too. Any other view without a panel is a failure, not a pass.
 */
const NO_PANEL = (row, width) => row === "devices" || row.startsWith("scenes") || (row === "effects" && width <= PHONE_MAX);

const only = flag("--views")?.split(",");
const rows = VIEWS.filter((v) => (!only || only.some((o) => v.name === o || v.name.startsWith(`${o}-`))) && !(hoverTips && v.overlay));

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".txt": "text/plain; charset=utf-8",
  ".ico": "image/x-icon",
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
  res.writeHead(200, { "content-type": MIME[path.extname(file)] ?? "application/octet-stream" });
  fs.createReadStream(file).pipe(res);
});

/**
 * The planted overflows of `--mutate-overflow`, one per measuring path. Each mode puts one thing
 * into the page that the check has to report, as the kind named in EXPECT, and PLANT returns the
 * function that takes it out again (so one page load serves every mode). A mode that finds nothing
 * to plant in returns null.
 *
 *   box          a 40 px wider element in the first drawn box (the box path)
 *   outside      an element in `.content`, outside every box, 40 px past the window's right edge
 *   outside-left the same, 40 px past the window's left edge
 *   panel        a `.panel` in `.content` whose right edge is 40 px past the window
 *   wide         a `.panel` 40 px wider than the window
 *   doc          the body 40 px wider than the window, so the document scrolls sideways
 *   scroll-list  a non-panel `overflow-y: auto` list inside the first box with an element 40 px
 *                past the box's right edge (the old scroll-x exemption hid this)
 *   scrolls      a `.panel` that scrolls sideways itself
 *   wrapped      a figure whose unit sits under it
 *   portal       a listbox in `body` 40 px past the box its trigger sits in
 *   canvas-left  a `canvas.stage-canvas` in its own clipping `.stage`, 40 px past the window's left
 *                edge (KNOWN_CLIPPED excuses the right edge only)
 *   canvas-unclipped
 *                the same canvas 40 px past the right edge in a `.stage` that does not clip
 *   stage-sibling
 *                a clipping `.stage` that holds a `canvas.stage-canvas` and a sibling 40 px past the
 *                window's right edge (the stage's clip excuses the canvas, not the sibling)
 *   pane         a scroll pane (the first class of FIT_PANES) 40 px high that holds a 120 px panel,
 *                so it scrolls and cuts the panel (desktop sizes only: a phone scrolls the document)
 *
 * Self-contained: it runs in the page (its source is sent there).
 */
const PLANT = (mode, { BOX, FIT_PANES }) => {
  const undo = [];
  const vw = document.documentElement.clientWidth;
  const drawn = (b) => b.getBoundingClientRect().width > 0 && b.checkVisibility({ visibilityProperty: true, opacityProperty: true });
  const content = document.querySelector(".content") ?? document.body;
  const make = (tag, cls, css) => {
    const el = document.createElement(tag);
    el.className = cls;
    el.style.cssText = css;
    return el;
  };
  const add = (parent, el) => {
    parent.appendChild(el);
    undo.push(() => el.remove());
    return el;
  };
  // A box that holds the planted element has to be a containing block, or the element would escape
  // the box's own clip (an absolute element is clipped only by ancestors that contain it).
  const positioned = (el) => {
    if (getComputedStyle(el).position !== "static") return;
    const before = el.style.position;
    el.style.position = "relative";
    undo.push(() => {
      el.style.position = before;
    });
  };
  // Puts `el` (absolute, in `parent`) so its left edge, or its right edge, sits at an x in the window.
  const placeAt = (el, parent, { left, right }) => {
    positioned(parent);
    el.style.position = "absolute";
    el.style.top = "0px";
    el.style.left = "0px";
    add(parent, el);
    const r = el.getBoundingClientRect();
    el.style.left = `${(left !== undefined ? left : right - r.width) - r.left}px`;
  };
  const firstDrawn = (sel) => [...document.querySelectorAll(sel)].find(drawn);
  const small = "height:4px;pointer-events:none";

  switch (mode) {
    case "box": {
      const box = firstDrawn(BOX);
      if (!box) return null;
      positioned(box);
      add(box, make("div", "ovf-mutant", `position:absolute;top:0;left:0;${small};width:calc(100% + 40px)`));
      return () => undo.reverse().forEach((f) => f());
    }
    case "outside":
      placeAt(make("div", "ovf-mutant", `width:100px;${small}`), content, { right: vw + 40 });
      return () => undo.reverse().forEach((f) => f());
    case "outside-left":
      placeAt(make("div", "ovf-mutant", `width:100px;${small}`), content, { left: -40 });
      return () => undo.reverse().forEach((f) => f());
    case "panel":
      placeAt(make("div", "ovf-mutant panel", `width:100px;${small};padding:0;border:0;min-height:0`), content, { right: vw + 40 });
      return () => undo.reverse().forEach((f) => f());
    case "wide":
      placeAt(make("div", "ovf-mutant panel", `width:${vw + 40}px;${small};padding:0;border:0;min-height:0`), content, { left: 0 });
      return () => undo.reverse().forEach((f) => f());
    case "doc": {
      const before = document.body.style.minWidth;
      document.body.style.minWidth = `${vw + 40}px`;
      undo.push(() => {
        document.body.style.minWidth = before;
      });
      return () => undo.reverse().forEach((f) => f());
    }
    case "scroll-list": {
      const panel = firstDrawn(BOX);
      if (!panel) return null;
      positioned(panel);
      const list = add(panel, make("div", "ovf-mutant-list", "overflow-y:auto;height:20px;width:100%;max-width:100%;flex:none"));
      const item = add(list, make("div", "ovf-mutant", `position:relative;${small};width:10px;flex:none`));
      const gap = panel.getBoundingClientRect().right - list.getBoundingClientRect().right;
      item.style.width = `${list.getBoundingClientRect().width + gap + 40}px`;
      return () => undo.reverse().forEach((f) => f());
    }
    case "scrolls": {
      const panel = make("div", "ovf-mutant panel", "width:200px;height:30px;overflow:auto;padding:0;border:0;min-height:0");
      placeAt(panel, content, { left: 0 });
      add(panel, make("div", "", `${small};width:320px;flex:none`));
      return () => undo.reverse().forEach((f) => f());
    }
    case "wrapped": {
      const holder = make("div", "", "width:80px");
      const input = make("input", "num", "display:block;width:60px;height:20px");
      const unit = make("span", "unit ovf-mutant", "display:block;width:30px;height:16px");
      unit.textContent = "°C";
      holder.append(input, unit);
      placeAt(holder, content, { left: 0 });
      return () => undo.reverse().forEach((f) => f());
    }
    case "portal": {
      // The box with the most room to its right, so the planted list can stay inside the window.
      const panels = [...document.querySelectorAll(BOX)].filter(drawn);
      if (panels.length === 0) return null;
      const room = (p) => vw - p.getBoundingClientRect().right;
      const panel = panels.sort((a, b) => room(b) - room(a))[0];
      positioned(panel);
      const trigger = make("button", "ovf-mutant-trigger", "position:absolute;top:0;left:0;width:10px;height:10px;padding:0;border:0");
      trigger.setAttribute("aria-controls", "ovf-mutant-list");
      trigger.setAttribute("aria-expanded", "true");
      trigger.setAttribute("aria-haspopup", "listbox");
      add(panel, trigger);
      const list = make("ul", "listbox ovf-mutant", "position:fixed;top:0;left:0;width:120px;height:20px;margin:0;opacity:1;animation:none;transition:none;transform:none;pointer-events:none");
      list.id = "ovf-mutant-list";
      list.setAttribute("role", "listbox");
      add(document.body, list);
      const r = list.getBoundingClientRect();
      list.style.left = `${panel.getBoundingClientRect().right + 40 - r.width - r.left}px`;
      return () => undo.reverse().forEach((f) => f());
    }
    case "canvas-left":
    case "canvas-unclipped":
    case "stage-sibling": {
      const stage = make("div", "stage ovf-mutant-stage", `width:200px;height:20px;aspect-ratio:auto;border:0;overflow:${mode === "canvas-unclipped" ? "visible" : "hidden"}`);
      placeAt(stage, content, { left: 0 });
      const canvas = make("canvas", "ovf-mutant stage-canvas", `position:absolute;top:0;height:4px;pointer-events:none;${
        mode === "canvas-left" ? "left:-40px;width:100px" : mode === "canvas-unclipped" ? `left:0;width:${vw + 40}px` : "left:0;width:100px"
      }`);
      add(stage, canvas);
      if (mode === "stage-sibling") add(stage, make("div", "ovf-mutant", `position:absolute;top:8px;left:0;${small};width:${vw + 40}px`));
      return () => undo.reverse().forEach((f) => f());
    }
    case "pane": {
      const cls = FIT_PANES.split(",")[0].trim().replace(/^\./, "");
      const pane = make("div", `ovf-mutant ${cls}`, "width:200px;height:40px;overflow-y:auto;flex:none;margin:0;padding:0");
      placeAt(pane, content, { left: 0 });
      add(pane, make("div", "panel", "height:120px;width:100%;flex:none;margin:0;padding:0;border:0;min-height:0"));
      return () => undo.reverse().forEach((f) => f());
    }
    default:
      return null;
  }
};

/** The kind of hit that proves each planted mode was seen (the hit's element must be the planted one). */
const EXPECT = {
  box: "box",
  outside: "window",
  "outside-left": "window-left",
  panel: "window",
  wide: "wide",
  doc: "doc",
  "scroll-list": "box",
  scrolls: "scrolls",
  wrapped: "wrapped",
  portal: "portal",
  "canvas-left": "window-left",
  "canvas-unclipped": "window",
  "stage-sibling": "window",
  pane: "scrolls-y",
};

/**
 * The one element the window check lets a clip excuse: Layout's `canvas.stage-canvas` is the
 * zoomed view, drawn wider than its stage on purpose and cut off by that stage. It is excused only
 * for its right edge, and only while its own `.stage` clips it and sits inside the window. Any
 * other clip excuses nothing (`.content { overflow: hidden }` hides an overflow, it does not make
 * it right), and a sibling in the same stage is measured like any other element.
 */
const KNOWN_CLIPPED = "canvas.stage-canvas";

/** Runs in the page: everything wrong with the edges. */
const MEASURE = ({ slack, extraBoxes, KNOWN_CLIPPED, FIT_PANES, PHONE_MAX }) => {
  const BOX = [".panel", ".rule", extraBoxes].filter(Boolean).join(", ");
  const vw = document.documentElement.clientWidth;
  // A box that is in the page but not drawn (a skeleton kept for its height, `visibility: hidden`)
  // is not a box to measure, nor to put the mutation in.
  const drawnBox = (b) => b.getBoundingClientRect().width > 0 && b.checkVisibility({ visibilityProperty: true, opacityProperty: true });
  const name = (el) => {
    const cls = String(el.getAttribute("class") ?? "").trim().split(/\s+/).filter(Boolean).slice(0, 2).join(".");
    return `${el.tagName.toLowerCase()}${cls ? `.${cls}` : ""}`;
  };
  const text = (el) => (el.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40);
  const round = (n) => Math.round(n * 10) / 10;

  // Which rules say, on purpose, that a box scrolls sideways: an `overflow-x: auto|scroll` (alone or
  // as the first value of a two-value `overflow`). A one-value `overflow: auto` is how a vertical
  // list is written and computes `overflow-x: auto` too, and so does `overflow-y: auto` alone, so
  // neither is a sideways scroller: both of those hide a too-wide row instead of showing it.
  const scrollRules = [];
  const sidewaysDecl = (st) => {
    const ox = st.getPropertyValue("overflow-x");
    if (ox !== "auto" && ox !== "scroll") return false;
    const both = st.getPropertyValue("overflow").trim();
    return !(both && !/\s/.test(both));
  };
  const walk = (rules) => {
    for (const r of rules) {
      if (r.selectorText !== undefined && r.style) {
        if (sidewaysDecl(r.style)) scrollRules.push(r.selectorText);
      } else if (r.cssRules) {
        // A media rule that does not apply here has no say.
        if (r.media && !matchMedia(r.media.mediaText).matches) continue;
        walk(r.cssRules);
      }
    }
  };
  for (const sheet of document.styleSheets) {
    try {
      walk(sheet.cssRules);
    } catch {
      /* a sheet from another origin: none in the export */
    }
  }
  const declaresSideways = (el) => sidewaysDecl(el.style) || scrollRules.some((sel) => {
    try {
      return el.matches(sel);
    } catch {
      return false;
    }
  });
  // A scroll container that is meant to scroll on x is meant to be wider than what it shows. A
  // `.panel` is never one of them: it is the thing being measured.
  const scrollsX = (el) => {
    if (el.matches(".panel")) return false;
    const o = getComputedStyle(el).overflowX;
    return (o === "auto" || o === "scroll") && declaresSideways(el);
  };
  // Not drawn, or drawn for a screen reader only: not an overflow. A tooltip waits at opacity 0
  // until the pointer is over its mark.
  const notDrawn = (el) => {
    const cs = getComputedStyle(el);
    return cs.display === "none" || cs.visibility === "hidden" || cs.opacity === "0";
  };
  /**
   * Is `el` exempt: not drawn, fixed (when asked), or inside a scroller that scrolls on x on
   * purpose and whose own right edge is within `limit` (a scroller that itself passes the edge is
   * the offender, and is measured as an element in its own right).
   */
  const skipped = (el, stop, { fixed, limit }) => {
    for (let a = el; a && a !== stop; a = a.parentElement) {
      if (notDrawn(a)) return true;
      if (fixed && getComputedStyle(a).position === "fixed") return true;
      if (a !== el && scrollsX(a) && a.getBoundingClientRect().right <= limit) return true;
    }
    return false;
  };
  // The one element the window check lets a clip excuse (see KNOWN_CLIPPED): its own stage (the
  // nearest `.stage`) has to clip it and has to sit inside the window itself, so what the canvas
  // draws past the window is what the stage cuts off. Only the right edge is excused; a canvas past
  // the left edge, and anything else in the stage, is measured.
  const clippedByItsStage = (el, kind) => {
    if (kind !== "window" || !el.matches(KNOWN_CLIPPED)) return false;
    const stage = el.closest(".stage");
    if (!stage || getComputedStyle(stage).overflowX === "visible") return false;
    const sr = stage.getBoundingClientRect();
    return sr.left >= -slack && sr.right <= vw + slack;
  };
  const tooSmall = (r) => r.width <= 2 || r.height <= 2;
  const hits = [];

  // 1. Elements past the right edge of their box, and boxes past the box that holds them.
  const boxes = [...document.querySelectorAll(BOX)].filter(drawnBox);
  const boxOf = new Map();
  for (const box of boxes) {
    const br = box.getBoundingClientRect();
    for (const el of box.querySelectorAll("*")) {
      if (el.closest(BOX) !== box) continue;
      const r = el.getBoundingClientRect();
      if (tooSmall(r) || r.right <= br.right + slack) continue;
      if (skipped(el, box, { fixed: true, limit: br.right + slack })) continue;
      boxOf.set(el, box);
    }
    // The box itself against the box above it (a `.rule` against its panel).
    const parent = box.parentElement?.closest(BOX);
    if (parent) {
      const pr = parent.getBoundingClientRect();
      if (br.right > pr.right + slack && !skipped(box, parent, { fixed: true, limit: pr.right + slack })) boxOf.set(box, parent);
    }
    // A box that scrolls sideways is a box whose content is wider than it.
    const ox = getComputedStyle(box).overflowX;
    if ((ox === "auto" || ox === "scroll") && box.scrollWidth > box.clientWidth + slack) {
      hits.push({ kind: "scrolls", el: name(box), text: "", over: box.scrollWidth - box.clientWidth });
    }
  }
  for (const el of [...boxOf.keys()].filter((e) => !(e.parentElement && boxOf.has(e.parentElement)))) {
    const over = el.getBoundingClientRect().right - boxOf.get(el).getBoundingClientRect().right;
    hits.push({ kind: "box", el: name(el), box: name(boxOf.get(el)), text: text(el), over: round(over) });
  }

  // 2. Anything past the window on either side, in a panel or not (`.content` clips what passes
  // the window, which hides it: that is no excuse), and panels wider than the window.
  const past = new Map();
  for (const el of document.querySelectorAll("body *")) {
    const r = el.getBoundingClientRect();
    if (tooSmall(r)) continue;
    const right = r.right > vw + slack;
    const left = r.left < -slack;
    if (!right && !left) continue;
    if (skipped(el, document.body, { fixed: false, limit: vw + slack })) continue;
    const kind = right ? "window" : "window-left";
    if (clippedByItsStage(el, kind)) continue;
    past.set(el, kind);
  }
  for (const [el, kind] of [...past].filter(([e]) => !(e.parentElement && past.has(e.parentElement)))) {
    const r = el.getBoundingClientRect();
    hits.push({ kind, el: name(el), text: text(el), over: round(kind === "window" ? r.right - vw : -r.left) });
  }
  for (const p of document.querySelectorAll(".panel")) {
    const w = p.getBoundingClientRect().width;
    if (w > vw + slack) hits.push({ kind: "wide", el: name(p), text: "", over: round(w - vw) });
  }
  const sw = document.documentElement.scrollWidth;
  if (sw > vw + slack) hits.push({ kind: "doc", el: "html", text: "", over: sw - vw });

  // 3. A unit that has wrapped under its figure: its middle is below the figure's bottom.
  for (const input of document.querySelectorAll("input.num")) {
    const unit = input.parentElement?.querySelector(".unit");
    if (!unit || unit.getBoundingClientRect().width === 0 || skipped(input, document.body, { fixed: true, limit: vw + slack })) continue;
    const ir = input.getBoundingClientRect();
    const ur = unit.getBoundingClientRect();
    if (ur.top + ur.height / 2 > ir.bottom) {
      hits.push({ kind: "wrapped", el: name(unit), text: text(unit), over: round(ur.top + ur.height / 2 - ir.bottom) });
    }
  }

  // 4. Portals. A popover (Popover.tsx) and a select's list (Select.tsx) are drawn into `body`, so
  // the box walk above never sees them: against the window, on all four sides (they place
  // themselves 8 px inside it), and against the box (a panel, a note, a row) the control that opened them sits in. That
  // control is known from `aria-controls` (a Select's list and, since #285, a Popover's panel), or,
  // for a portal with no id, when exactly one control that has a popup is open. When the control
  // cannot be found the box check does not run, and the row says UNKNOWN for that portal; the
  // window check still ran. (PageHeader.tsx's portals go into the status
  // strip, inside the document, so the walk above measures them.)
  const vh = document.documentElement.clientHeight;
  const portals = [...document.querySelectorAll(".popover, .listbox")].filter(
    (p) => p.parentElement === document.body && drawnBox(p) && p.getAttribute("aria-hidden") !== "true",
  );
  const openers = [...document.querySelectorAll('[aria-expanded="true"][aria-haspopup]')];
  const unknowns = [];
  for (const p of portals) {
    const r = p.getBoundingClientRect();
    if (r.left < -slack) hits.push({ kind: "window-left", el: name(p), text: text(p), over: round(-r.left) });
    if (r.top < -slack) hits.push({ kind: "window-top", el: name(p), text: text(p), over: round(-r.top) });
    if (r.bottom > vh + slack) hits.push({ kind: "window-bottom", el: name(p), text: text(p), over: round(r.bottom - vh) });
    const trigger = p.id ? document.querySelector(`[aria-controls="${p.id}"]`) : openers.length === 1 ? openers[0] : null;
    if (!trigger) {
      // The box check could not run: say so, it is not a pass.
      unknowns.push({ el: name(p), text: text(p), why: p.id ? `no control has aria-controls="${p.id}"` : `${openers.length} open aria-haspopup controls, no aria-controls` });
      continue;
    }
    const panel = trigger.closest(BOX);
    if (panel && drawnBox(panel)) {
      const pr = panel.getBoundingClientRect();
      if (r.right > pr.right + slack) {
        hits.push({ kind: "portal", el: name(p), box: name(panel), from: name(trigger), text: text(p), over: round(r.right - pr.right) });
      }
    }
  }

  // 5. The fit-window rule (#294): on a desktop window every view fits, and a scroll pane is only
  // where R1 puts what is taller than a short window. A pane that scrolls (or clips) on a desktop
  // size is a view that does not fit; each panel it cuts off at the bottom is named. A phone
  // scrolls the document and has no pane.
  if (vw > PHONE_MAX) {
    for (const pane of document.querySelectorAll(FIT_PANES)) {
      if (!drawnBox(pane) || getComputedStyle(pane).overflowY === "visible") continue;
      const over = pane.scrollHeight - pane.clientHeight;
      if (over <= slack) continue;
      hits.push({ kind: "scrolls-y", el: name(pane), text: text(pane.querySelector(".panel-head, h2, h3") ?? pane), over, shown: pane.clientHeight });
      const edge = pane.getBoundingClientRect().top + pane.clientTop + pane.clientHeight;
      for (const panel of pane.querySelectorAll(".panel")) {
        if (panel.parentElement?.closest(".panel") && pane.contains(panel.parentElement.closest(".panel"))) continue;
        if (!drawnBox(panel)) continue;
        const cut = panel.getBoundingClientRect().bottom - edge;
        if (cut > slack) hits.push({ kind: "panel-cut", el: name(panel), text: text(panel.querySelector(".panel-head, h2, h3") ?? panel), pane: name(pane), over: round(cut) });
      }
    }
  }
  return {
    vw,
    boxes: boxes.length,
    panels: boxes.filter((b) => b.matches(".panel")).length,
    portals: portals.length,
    unknowns,
    hits,
  };
};

/**
 * One page call: the measure with nothing planted, then, for each mode asked for, plant it,
 * measure and take it out again. Planting and measuring share one call so the view cannot redraw
 * between them. Built from the source of PLANT and MEASURE, so the page runs the same code.
 */
const RUN = new Function(
  "opts",
  `const PLANT = ${PLANT.toString()};
   const MEASURE = ${MEASURE.toString()};
   const base = MEASURE(opts);
   const BOX = [".panel", ".rule", opts.extraBoxes].filter(Boolean).join(", ");
   const planted = {};
   for (const mode of opts.modes) {
     const undo = PLANT(mode, { slack: opts.slack, BOX, FIT_PANES: opts.FIT_PANES });
     if (!undo) { planted[mode] = null; continue; }
     try { planted[mode] = MEASURE(opts); } finally { undo(); }
   }
   return { base, planted };`,
);

const describe = (x) => {
  switch (x.kind) {
    case "box":
      return `${x.el} "${x.text}" passes ${x.box} by ${x.over}px`;
    case "window":
      return `${x.el} "${x.text}" passes the window's right edge by ${x.over}px`;
    case "window-left":
      return `${x.el} "${x.text}" passes the window's left edge by ${x.over}px`;
    case "window-top":
      return `${x.el} "${x.text}" passes the window's top edge by ${x.over}px`;
    case "window-bottom":
      return `${x.el} "${x.text}" passes the window's bottom edge by ${x.over}px`;
    case "clipped":
      return `${x.el} "${x.text}" is cut off by ${x.clipper} at its ${x.edge} by ${x.over}px`;
    case "portal":
      return `${x.el} "${x.text}" (opened from ${x.from}) passes ${x.box} by ${x.over}px`;
    case "wide":
      return `${x.el} is ${x.over}px wider than the window`;
    case "doc":
      return `the document scrolls sideways by ${x.over}px`;
    case "scrolls":
      return `${x.el} scrolls sideways: its content is ${x.over}px wider than it`;
    case "scrolls-y":
      return `${x.el} ("${x.text}") scrolls vertically at a desktop size: its content is ${x.over}px taller than the ${x.shown}px it shows (the fit-window rule: no scroll)`;
    case "panel-cut":
      return `${x.el} "${x.text}" ends ${x.over}px below the bottom of ${x.pane}, so it is cut off until the pane scrolls`;
    case "wrapped":
      return `${x.el} "${x.text}" has wrapped under its figure, ${x.over}px below the figure's bottom`;
    default:
      return JSON.stringify(x);
  }
};

/** Runs in the page: every drawn control that has a tooltip, in DOM order. */
const TIPS = () => {
  const drawn = (el) => {
    const cs = getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden") return false;
    const r = el.getBoundingClientRect();
    return r.width > 1 && r.height > 1;
  };
  return [...document.querySelectorAll(".tip")]
    .map((tip, index) => {
      if (!drawn(tip)) return null;
      const b = tip.querySelector(".tip-bubble");
      // A bubble its own CSS never draws (display: none) has no placement to check.
      if (!b || !drawn(b)) return null;
      return { index, text: (b.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40) };
    })
    .filter(Boolean);
};

/** Runs in the page after the hover: the shown bubble against the window and its clippers. */
const MEASURE_TIP = ({ index, slack }) => {
  const tip = document.querySelectorAll(".tip")[index];
  const b = tip?.querySelector(".tip-bubble");
  if (!b) return { missing: true };
  const name = (el) => {
    const cls = String(el.getAttribute("class") ?? "").trim().split(/\s+/).filter(Boolean).slice(0, 2).join(".");
    return `${el.tagName.toLowerCase()}${cls ? `.${cls}` : ""}`;
  };
  const round = (n) => Math.round(n * 10) / 10;
  const r = b.getBoundingClientRect();
  const vw = document.documentElement.clientWidth;
  const vh = document.documentElement.clientHeight;
  const text = (b.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40);
  const hits = [];
  if (r.right > vw + slack) hits.push({ kind: "window", el: name(b), text, over: round(r.right - vw) });
  if (r.left < -slack) hits.push({ kind: "window-left", el: name(b), text, over: round(-r.left) });
  if (r.bottom > vh + slack) hits.push({ kind: "window-bottom", el: name(b), text, over: round(r.bottom - vh) });
  if (r.top < -slack) hits.push({ kind: "window-top", el: name(b), text, over: round(-r.top) });
  // An ancestor that clips the bubble cuts its words off even where the window has room.
  for (let a = b.parentElement; a && a !== document.body; a = a.parentElement) {
    const cs = getComputedStyle(a);
    if (cs.overflowX === "visible" && cs.overflowY === "visible") continue;
    const ar = a.getBoundingClientRect();
    const cuts = [];
    if (cs.overflowX !== "visible") {
      if (r.right > ar.right + slack) cuts.push(["right", r.right - ar.right]);
      if (r.left < ar.left - slack) cuts.push(["left", ar.left - r.left]);
    }
    if (cs.overflowY !== "visible") {
      if (r.bottom > ar.bottom + slack) cuts.push(["bottom", r.bottom - ar.bottom]);
      if (r.top < ar.top - slack) cuts.push(["top", ar.top - r.top]);
    }
    if (cuts.length) {
      const worst = cuts.sort((x, y) => y[1] - x[1])[0];
      hits.push({ kind: "clipped", el: name(b), clipper: name(a), edge: worst[0], over: round(worst[1]), text });
      break;
    }
  }
  return { opacity: Number(getComputedStyle(b).opacity), hits, text };
};

/** Hovers every tooltip the page draws, and measures each bubble once it is shown. */
async function hoverEveryTip(page) {
  const tips = await page.evaluate(TIPS);
  const hits = [];
  let hovered = 0;
  for (const tip of tips) {
    try {
      await page.locator(".tip").nth(tip.index).hover({ timeout: 10000 });
    } catch (e) {
      hits.push(`"${tip.text}": could not hover it (${String(e.message).split("\n")[0]})`);
      continue;
    }
    try {
      await page.waitForFunction(
        (i) => {
          const b = document.querySelectorAll(".tip")[i]?.querySelector(".tip-bubble");
          return b ? Number(getComputedStyle(b).opacity) > 0.99 : false;
        },
        tip.index,
        { timeout: 4000 },
      );
    } catch {
      // Measured below anyway: a bubble that never showed is reported as its own failure.
    }
    const m = await page.evaluate(MEASURE_TIP, { index: tip.index, slack: SLACK_PX });
    if (m.missing) {
      hits.push(`"${tip.text}": its bubble left the page while it was hovered`);
      continue;
    }
    if (m.opacity < 0.99) hits.push(`"${m.text}": hovered but the bubble did not show (opacity ${m.opacity})`);
    else hovered++;
    for (const h of m.hits) hits.push(describe(h));
  }
  const skipped = await page.evaluate(() => document.querySelectorAll(".tip").length);
  return { tips: tips.length, skipped: skipped - tips.length, hovered, hits };
}

/** A busy machine can time a request out: up to three tries, from a blank page. */
async function goto(page, url) {
  for (let attempt = 1; ; attempt++) {
    try {
      await page.goto(url, { waitUntil: "load", timeout: 30000 });
      return;
    } catch (e) {
      if (attempt === 3) throw e;
      // Back to a blank page, and wait for it to have loaded, so the next try starts from rest.
      await page.goto("about:blank").catch(() => {});
      await page.waitForLoadState("load").catch(() => {});
    }
  }
}

/**
 * Wait until the view is drawn and still: fonts ready, the anchor on screen, no load frame
 * waiting, every animation that ends has ended (the ones that loop, a spinner, may run), and two
 * frames since.
 */
async function settle(page, anchor) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForFunction(
    (sel) => {
      // Any one of them drawn: the first match can be a sheet that is shut on a phone.
      const drawn = [...document.querySelectorAll(sel)].some((el) => {
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      });
      if (!drawn) return false;
      if (document.querySelector('[data-state="loading"], [aria-busy="true"]')) return false;
      return document.getAnimations().every((a) => a.playState === "finished" || a.effect?.getTiming().iterations === Infinity);
    },
    anchor,
    { timeout: T },
  );
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
}

async function main() {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  let browser;
  let channel = "chromium";
  try {
    browser = await chromium.launch({ headless: true });
  } catch {
    // No downloaded Chromium (a CI runner): the Chrome that comes with the runner will do.
    channel = "chrome";
    browser = await chromium.launch({ headless: true, channel: "chrome" });
  }
  console.log(`overflow-check: browser channel ${channel}, ${rows.length} views x ${SIZES.length} sizes x ${SCHEMES.length} schemes`);
  if (hoverTips) console.log("overflow-check: HOVER mode: every drawn tooltip is hovered and its shown bubble measured");
  if (mutate) console.log(`overflow-check: MUTATION on, planted in every row, one at a time: ${mutateModes.join(", ")}: every row must report each`);
  const failures = [];
  let checks = 0;
  const caught = Object.fromEntries(mutateModes.map((m) => [m, 0]));
  const missed = Object.fromEntries(mutateModes.map((m) => [m, 0]));
  let unknownTotal = 0;
  let tipTotal = 0;
  let hoverTotal = 0;
  try {
    for (const [w, h] of SIZES) {
      for (const scheme of SCHEMES) {
        const ctx = await browser.newContext({
          viewport: { width: w, height: h },
          colorScheme: scheme,
          isMobile: w <= PHONE_MAX,
          hasTouch: w <= PHONE_MAX,
        });
        await ctx.addInitScript((s) => {
          try {
            localStorage.setItem("canvas-rgb.theme", s);
            localStorage.setItem("canvas-rgb.demo-onboarded", "1");
            // "Later" in this launch: the first-run onboarding would take the whole window.
            sessionStorage.setItem("canvas-rgb.launch", "overflow-check");
            localStorage.setItem("canvas-rgb.onboarding-later", "overflow-check");
          } catch {
            /* storage blocked */
          }
        }, scheme);
        const page = await ctx.newPage();
        for (const v of rows) {
          const label = `${w}x${h} ${scheme} ${v.name}`;
          const known = NO_PANEL(v.name, w);
          const extraBoxes = v.boxes ?? "";
          // A known no-panel view is anchored on its own rows; any other on a panel.
          const anchor = known ? extraBoxes : ".panel";
          const bad = [];
          const unknown = [];
          // A phone scrolls the document, so no pane is planted on it (and none is expected).
          const rowModes = mutateModes.filter((mode) => mode !== "pane" || w > PHONE_MAX);
          let m = null;
          let detail = "-";
          let mutantNote = "";
          try {
            await goto(page, `${base}${v.url}`);
            // `before`: what the page draws before the row's clicks (the Effects gallery on a phone
            // has no panel until an effect is opened).
            await settle(page, v.prep && v.before ? v.before : anchor);
            if (v.prep) {
              await v.prep(page);
              await settle(page, anchor);
            }
            if (hoverTips) {
              const h = await hoverEveryTip(page);
              tipTotal += h.tips;
              hoverTotal += h.hovered;
              detail = `${h.tips} tips, ${h.hovered} hovered, ${h.skipped} not drawn`;
              for (const b of h.hits) bad.push(b);
              if (h.tips === 0) bad.push("nothing was measured: the view draws no tooltip");
            } else {
              const run = await page.evaluate(RUN, { slack: SLACK_PX, extraBoxes, KNOWN_CLIPPED, FIT_PANES, PHONE_MAX, modes: rowModes });
              m = run.base;
              detail = `${m.panels} panels, ${m.boxes - m.panels} other boxes, ${m.portals} portals`;
              for (const u of m.unknowns) unknown.push(`UNKNOWN: ${u.el} "${u.text}" was not measured against its opening control's box (${u.why}); its window edges were`);
              if (m.boxes === 0) bad.push("nothing was measured: no box drawn");
              else if (m.panels === 0 && !known) bad.push("nothing was measured: no .panel drawn, and the view is not on the NO_PANEL list");
              if (mutate) {
                // Each planted overflow has to come back as the kind of hit its path reports, on the
                // element that was planted (the document has no such element: it has to grow).
                const was = m.hits.find((x) => x.kind === "doc")?.over ?? 0;
                const got = [];
                const lost = [];
                for (const mode of rowModes) {
                  const p = run.planted[mode];
                  const kind = EXPECT[mode];
                  const seen = !p ? false : p.hits.some((x) => x.kind === kind && (kind === "doc" ? x.over >= was + 30 : x.el.includes("ovf-mutant")));
                  if (seen) {
                    caught[mode]++;
                    got.push(mode);
                  } else {
                    missed[mode]++;
                    lost.push(mode);
                    bad.push(p ? `MUTATION MISSED (${mode}): not reported as ${kind}` : `MUTATION MISSED (${mode}): nothing to plant it in`);
                  }
                }
                detail += `, ${m.hits.length} real hits (not counted)`;
                mutantNote = `caught ${got.length} of ${rowModes.length}: ${got.join(", ") || "none"}${lost.length ? `; NOT caught: ${lost.join(", ")}` : ""}`;
              } else {
                for (const x of m.hits) bad.push(describe(x));
              }
            }
            if (shotsDir) {
              fs.mkdirSync(shotsDir, { recursive: true });
              await page.screenshot({ path: path.join(shotsDir, `${shotTag}-${v.name}-${w}x${h}-${scheme}.png`) });
            }
          } catch (e) {
            bad.push(`could not measure: ${String(e.message).split("\n")[0]}`);
          }
          checks++;
          unknownTotal += unknown.length;
          console.log(`${bad.length ? "FAIL" : unknown.length ? "unk " : "ok  "} ${label}  [${detail}]`);
          if (mutantNote) console.log(`       ~ ${mutantNote}`);
          for (const u of unknown) console.log(`       ? ${u}`);
          for (const b of bad) {
            console.log(`       - ${b}`);
            failures.push(`${label}: ${b}`);
          }
        }
        await ctx.close();
      }
    }
  } finally {
    await browser.close();
    server.close();
  }
  console.log(`\n${checks} view checks, ${failures.length} hits, ${unknownTotal} unknown (not measured, so not a pass)`);
  if (hoverTips) console.log(`${tipTotal} tooltips hovered (${hoverTotal} showed their bubble) over ${checks} rows`);
  if (mutate) {
    console.log("mutation, rows that reported the planted overflow:");
    for (const mode of mutateModes) console.log(`  ${mode.padEnd(13)} ${caught[mode]} of ${caught[mode] + missed[mode]} rows, missed in ${missed[mode]}`);
  }
  if (checks === 0 || failures.length) process.exitCode = 1;
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
