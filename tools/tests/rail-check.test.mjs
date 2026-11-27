// Regression test for #467, guard #478: the rail-check verdict is a real sideways scroll, not the
// computed overflow-x. `overflow-y: auto` makes overflow-x compute to auto even on a healthy rail,
// so a judge that trusted the computed value would fail every build; one that trusted only the
// declared value would miss the bug. judge() is exported from the tool and tested here with a
// synthetic measurement. Run with: node tools/tests/rail-check.test.mjs

import test from "node:test";
import assert from "node:assert/strict";
import { judge, SLACK } from "../rail-check.mjs";

function measurement(over = 0) {
  return {
    vw: 1024,
    vh: 768,
    scrollWidth: 63 + over,
    clientWidth: 63,
    overflowX: over,
    scrollHeight: 768,
    clientHeight: 768,
    overflowY: 0,
    // A vertical rail always computes this, fixed or not.
    computedOverflowX: "auto",
    computedOverflowY: "auto",
    rail: { left: 0, right: 63, width: 63 },
    fixedBubbles: 2,
    labels: 7,
  };
}

test("a rail that does not scroll sideways passes, even though overflow-x computes to auto", () => {
  assert.deepEqual(judge(measurement(0)), []);
  assert.deepEqual(judge(measurement(SLACK)), []);
});

test("a sideways-scrolling rail is a failure, named with the px cut", () => {
  const bad = judge(measurement(88));
  assert.equal(bad.length, 1);
  assert.ok(bad[0].includes("88px"), `expected the px cut in the failure, got: ${bad[0]}`);
  assert.ok(bad[0].includes("scrollWidth 151 > clientWidth 63"), `expected the widths, got: ${bad[0]}`);
});

test("a missing rail is a failure, not a pass", () => {
  const bad = judge({ error: "no nav.rail" });
  assert.deepEqual(bad, ["no nav.rail"]);
});
