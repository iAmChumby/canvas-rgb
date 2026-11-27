// Regression tests for the bar check's rules (#586, fix pass 1). judge() is imported without a
// browser, so these run in tools/tests/run-all.ps1. Run directly with:
//   node tools/tests/bar-check.test.mjs

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { judge, SIZES, ROUTES, STATES, LONG_NAME, USER_TITLE_FLOOR } from "../bar-check.mjs";

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

test("B11: the scene name is drawn once on a narrow strip", () => {
  const m = measurement();
  m.vw = 390;
  m.forms = { nowName: false, railValue: true };
  assert.ok(judge(m, state("scenes")).some((b) => b.includes("drawn once")));
});

test("B7: the user title measures a stable box and cuts on code points", () => {
  const tsx = source("app/web/src/shell/PageHeader.tsx");
  assert.match(tsx, /Array\.from\(title\)/, "name must be cut on code points, not UTF-16 units");
  assert.match(tsx, /titleBox\.getBoundingClientRect\(\)\.width/, "room must come from the title slot");
  assert.ok(!/el\.clientWidth/.test(tsx), "the shrinking heading must not measure its own room");
  assert.match(tsx, /ro\.observe\(titleBox\)/, "a slot that grows must grow the name back");
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
