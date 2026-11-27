// Regression tests for the bar check's rules (#586). judge() is imported without a browser, so
// these run in tools/tests/run-all.ps1 alongside the nav and rail checks. Run directly with:
//   node tools/tests/bar-check.test.mjs

import test from "node:test";
import assert from "node:assert/strict";
import { judge, SIZES, ROUTES, STATES, LONG_NAME, USER_TITLE_FLOOR } from "../bar-check.mjs";

const state = (name = "fans", value = "loaded") => ({ name, state: value });
const measurement = () => ({ clip: [], names: [], primaryPresent: true, userTitlePresent: false, docOverflow: 0 });

test("the check covers the seven sizes and every page state", () => {
  assert.deepEqual(SIZES.map((s) => `${s.w}x${s.h}`), [
    "1920x1040", "1440x900", "1280x800", "1024x768", "768x1024", "390x844", "360x780",
  ]);
  assert.equal(ROUTES.length, 10);
  assert.ok(STATES.some((s) => s.state === "loading"));
  assert.ok(STATES.some((s) => s.state === "palette"));
  assert.ok(STATES.some((s) => s.state === "popover"));
  assert.ok(STATES.some((s) => s.state === "long-name" && s.url.includes(encodeURIComponent(LONG_NAME))));
});

test("a clipped page title is a failure and names the title", () => {
  const m = measurement();
  m.clip.push({ selector: "h1", text: "Fan…", full: "Fans", need: 44, given: 43, title: true });
  const bad = judge(m, state());
  assert.ok(bad.some((b) => b.includes("title") && b.includes("Fan")), JSON.stringify(bad));
});

test("a clipped primary action is a failure and names the action", () => {
  const m = measurement();
  m.clip.push({ selector: "button", text: "Save", need: 80, given: 70, primary: true });
  const bad = judge(m, state());
  assert.ok(bad.some((b) => b.includes("primary action")), JSON.stringify(bad));
});

test("a clipped rail value or a sentence in the strip is a failure", () => {
  const m = measurement();
  m.clip.push({ selector: ".rail-value", text: "Everyth…", need: 80, given: 40, inRail: true });
  m.clip.push({ selector: ".strip-now-detail", text: "on 5 devic…", need: 86, given: 72 });
  const bad = judge(m, state());
  assert.ok(bad.some((b) => b.includes("rail value")));
  assert.ok(bad.some((b) => b.includes("on 5 devic")));
});

test("a user-named title may clip in the middle, but keeps its floor and its full name", () => {
  const long = "A".repeat(40);
  const ok = measurement();
  ok.userTitlePresent = true;
  ok.clip.push({ selector: "h1", text: `${"A".repeat(8)}…${"B".repeat(8)}`, full: long, need: 630, given: 200, userTitle: true });
  assert.deepEqual(judge(ok, state("builder", "long-name")), []);

  const short = measurement();
  short.userTitlePresent = true;
  short.clip.push({ selector: "h1", text: "AAAA", full: long, need: 630, given: 20, userTitle: true });
  const bad = judge(short, state("builder", "long-name"));
  assert.ok(bad.some((b) => b.includes("floor") && b.includes(String(USER_TITLE_FLOOR))), JSON.stringify(bad));

  const noCarrier = measurement();
  noCarrier.userTitlePresent = true;
  noCarrier.clip.push({ selector: "h1", text: `${"A".repeat(8)}…${"B".repeat(8)}`, full: "", need: 630, given: 200, userTitle: true });
  assert.ok(judge(noCarrier, state("builder", "long-name")).some((b) => b.includes("full name")));
});

test("Save leaving the bar is a failure in every state", () => {
  const m = measurement();
  m.primaryPresent = false;
  assert.ok(judge(m, state()).some((b) => b.includes("primary action left")));
});

test("an accessible name shorter than the visible words is a failure", () => {
  const m = measurement();
  m.names.push({ selector: ".select-trigger", name: "Audio source", visible: "Audio source: Voicemeeter Input" });
  assert.ok(judge(m, state("settings")).some((b) => b.includes("accessible name")));
});
