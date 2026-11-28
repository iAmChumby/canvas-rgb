// Regression tests for the bar check's rules (#586, fix pass 1). judge() is imported without a
// browser, so these run in tools/tests/run-all.ps1. Run directly with:
//   node tools/tests/bar-check.test.mjs

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { judge, SIZES, ROUTES, STATES, LONG_NAME, LONG_SCENE, USER_TITLE_FLOOR } from "../bar-check.mjs";

const source = (p) => readFileSync(path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", p), "utf8");

const state = (name = "fans", value = "loaded") => ({ name, state: value });
const measurement = () => ({
  vw: 1440, docOverflow: 0, docHidden: 0, clip: [], names: [], primaryPresent: true,
  userTitle: { present: false, visible: false, full: "", fullWidth: 0, shown: "", room: 0 },
  fixtureError: false, forms: { nowName: false, railValue: false },
});

test("the check covers the seven sizes, every route and the driven states", () => {
  assert.deepEqual(SIZES.map((s) => `${s.w}x${s.h}`), [
    "1920x1040", "1440x900", "1280x800", "1024x768", "768x1024", "390x844", "360x780",
  ]);
  assert.equal(ROUTES.length, 11);
  assert.ok(STATES.some((s) => s.state === "loading"));
  assert.ok(STATES.some((s) => s.state === "palette"));
  assert.ok(STATES.some((s) => s.state === "popover"));
  assert.ok(STATES.some((s) => s.state === "pending" && s.name === "peripherals"));
  assert.ok(STATES.some((s) => s.state === "long-name" && s.url.includes(encodeURIComponent(LONG_NAME))));
});

test("B1: sideways scroll and a page that must fit are failures", () => {
  const m = measurement();
  m.docOverflow = 7;
  assert.ok(judge(m, state()).some((b) => b.includes("sideways")));
  const n = measurement();
  n.vw = 390;
  n.docHidden = 7;
  assert.deepEqual(judge(n, state()), []);
  assert.ok(judge(n, state("layout")).some((b) => b.includes("down by 7px")));
});

test("a clipped page title is a failure and names the title", () => {
  const m = measurement();
  m.clip.push({ selector: "h1", text: "Fan…", full: "Fans", need: 44, given: 43, title: true });
  assert.ok(judge(m, state()).some((b) => b.includes("title") && b.includes("Fan")));
});

test("B2/B4: a hidden primary is a failure; an inline clip is measured", () => {
  const m = measurement();
  m.primaryPresent = false;
  assert.ok(judge(m, state()).some((b) => b.includes("not visible in the bar")));
  const n = measurement();
  n.clip.push({ selector: "span.strip-effect-name", text: "Gaming", need: 51, given: 8 });
  assert.ok(judge(n, state()).some((b) => b.includes("Gaming")));
});

test("a clipped rail value or a sentence in the strip is a failure", () => {
  const m = measurement();
  m.clip.push({ selector: ".rail-value", text: "Everyth…", need: 80, given: 40, inRail: true });
  m.clip.push({ selector: ".strip-now-detail", text: "on 5 devic…", need: 86, given: 72 });
  assert.ok(judge(m, state()).some((b) => b.includes("rail value")));
  assert.ok(judge(m, state()).some((b) => b.includes("on 5 devic")));
});

test("B5: a user title keeps its floor, its carrier and the room it has", () => {
  const long = "A".repeat(40);
  const ok = measurement();
  ok.userTitle = { present: true, visible: true, full: long, fullWidth: 630, shown: `${"A".repeat(8)}…${"B".repeat(8)}`, room: 200 };
  assert.deepEqual(judge(ok, state("builder", "long-name")), []);
  const roomy = measurement();
  roomy.userTitle = { present: true, visible: true, full: long, fullWidth: 630, shown: "A…B", room: 794 };
  assert.ok(judge(roomy, state("builder", "long-name")).some((b) => b.includes("fits the 794px room")));
  const short = measurement();
  short.userTitle = { present: true, visible: true, full: long, fullWidth: 630, shown: "AAAAA…BBBBB", room: 200 };
  assert.ok(judge(short, state("builder", "long-name")).some((b) => b.includes("floor") && b.includes(String(USER_TITLE_FLOOR))));
  const missing = measurement();
  missing.userTitle = { present: true, visible: true, full: "", fullWidth: 0, shown: "A…B", room: 200 };
  assert.ok(judge(missing, state("builder", "long-name")).some((b) => b.includes("full name")));
});

test("B6: a builder fixture that shows the error panel is a failure", () => {
  const m = measurement();
  m.fixtureError = true;
  assert.ok(judge(m, state("builder", "long-name")).some((b) => b.includes("fixture did not load")));
});

test("B11: the playing name is on screen exactly once where the rail is collapsed", () => {
  const m = measurement();
  m.vw = 390;
  m.playing = "Gaming";
  m.nameCopies = ["span.strip-effect-name"];
  assert.deepEqual(judge(m, state("devices")), []);
  m.nameCopies = ["span.strip-effect-name", "p.status-body-line"];
  assert.ok(judge(m, state("devices")).some((b) => b.includes("on screen 2 times")));
  m.nameCopies = [];
  assert.ok(judge(m, state("devices")).some((b) => b.includes("on screen 0 times")));
});

test("B11: the long scene name state must be on screen", () => {
  assert.ok(STATES.some((s) => s.state === "long-scene"));
  const m = measurement();
  m.playing = "Gaming";
  assert.ok(judge(m, state("scenes", "long-scene")).some((b) => b.includes("not the one playing")));
  m.playing = LONG_SCENE;
  assert.deepEqual(judge(m, state("scenes", "long-scene")), []);
});

test("B12: Pause and search off the title's line is a failure on a phone", () => {
  const m = measurement();
  m.vw = 360;
  m.controlsOff = 27;
  assert.ok(judge(m, state("builder")).some((b) => b.includes("left the title's line")));
  m.vw = 1440;
  assert.ok(!judge(m, state("builder")).some((b) => b.includes("left the title's line")));
});

test("B9/B13: context and row count come from one measured rule, not the window", () => {
  const tsx = source("app/web/src/shell/StatusStrip.tsx");
  const css = source("app/web/src/styles/app.css");
  assert.ok(!/window\.innerWidth/.test(tsx), "no viewport guess in the strip");
  assert.match(tsx, /dataset\.barContext/);
  assert.match(css, /:root\[data-bar-context="body"\] \.rd-context/);
  const narrow = css.match(/@container strip \(max-width: 1150px\)\s*\{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.ok(!/\.rd-context/.test(narrow), "no second 1150/1151 rule for context");
});

test("an accessible name shorter than the visible words is a failure", () => {
  const m = measurement();
  m.names.push({ selector: ".select-trigger", name: "Audio source", visible: "Audio source: Voicemeeter Input" });
  assert.ok(judge(m, state("settings")).some((b) => b.includes("accessible name")));
});

test("B10: every secondary key has one home, the bar or the overflow", () => {
  const overflow = state("builder", "overflow");
  const held = measurement();
  held.barActions = ["Revert Revert"];
  held.menuActions = ["Back to Case"];
  assert.deepEqual(judge(held, overflow), []);
  const lost = measurement();
  lost.barActions = ["Back to Case"];
  lost.menuActions = [];
  assert.ok(judge(lost, overflow).some((b) => b.includes("Revert is not reachable")));
  const twice = measurement();
  twice.barActions = ["Revert", "Back to Case"];
  twice.menuActions = ["Revert"];
  assert.ok(judge(twice, overflow).some((b) => b.includes("Revert is in the bar and in the overflow")));
});

test("a cut-off key or one part drawn over another is a failure", () => {
  const m = measurement();
  m.cutControls = ["Save the stack"];
  m.overlaps = ["span.strip-now over button.strip-mark"];
  assert.equal(judge(m, state()).length, 2);
});

test("B6: each driven state must be on screen", () => {
  for (const [name, value] of [["builder", "dirty"], ["builder", "renaming"], ["fans", "dirty"], ["scenes", "renaming"]]) {
    assert.ok(STATES.some((s) => s.name === name && s.state === value), `${name} ${value} is not driven`);
    assert.ok(judge(measurement(), state(name, value)).length > 0, `${name} ${value} passes with nothing on screen`);
  }
});

test("B7: a name that stays cut on a wide window is a failure", () => {
  const m = measurement();
  m.userTitle = { present: true, visible: true, full: LONG_NAME, fullWidth: 630, shown: "A very…hand", room: 100 };
  assert.ok(judge(m, state("builder", "regrow")).some((b) => b.includes("grow back")));
});

test("B8: status words with no home are a failure", () => {
  const m = measurement();
  m.statusLost = true;
  assert.ok(judge(m, state("devices")).some((b) => b.includes("no home")));
});

test("a phone says the unsent changes in words and keeps Revert in the overflow", () => {
  const m = measurement();
  m.vw = 390;
  m.pendingWords = false;
  m.barActions = ["Revert"];
  m.menuActions = [];
  const bad = judge(m, state("peripherals", "overflow"));
  assert.ok(bad.some((b) => b.includes("not said in words")));
  assert.ok(bad.some((b) => b.includes("not in the overflow menu on a phone")));
});
