// Regression tests for the hero glow's motion math (issues #15 and #63).
// Node's built-in runner: no test framework, no new dependency. Node 22 imports the pure
// ./src/components/glow.ts module directly (type stripping), so the test reads the same code
// the component runs.
import test from "node:test";
import assert from "node:assert/strict";
import {
  DT_MAX,
  MOVE_RATE,
  MOVE_SHARE,
  TAU_FOLLOW,
  easeK,
  moveCap,
} from "../src/components/glow.ts";

/** How long it takes a cap-limited light to cross `distance`, simulated at a given refresh rate. */
function sweepSeconds(distance, hz) {
  const dt = 1 / hz;
  let travelled = 0;
  let seconds = 0;
  // Enough frames to cross even at the lowest rate, with a wall-clock stop.
  for (let i = 0; i < hz * 5 && travelled < distance; i++) {
    travelled += moveCap(distance, dt);
    seconds += dt;
  }
  return travelled >= distance ? seconds : Infinity;
}

test("the follow ease closes the same share of the gap in the same time", () => {
  // The ease is exponential in elapsed time, so two 120 Hz frames (the same wall-clock time
  // as one 60 Hz frame) close exactly the same share: the input is time, not a frame count.
  const k60 = easeK(1 / 60, TAU_FOLLOW);
  const k120 = easeK(1 / 120, TAU_FOLLOW);
  assert.ok(Math.abs(k60 - (1 - (1 - k120) ** 2)) < 1e-9);
  // After three time constants about 95% of the gap is closed, at any refresh rate.
  assert.ok(Math.abs(easeK(3 * TAU_FOLLOW, TAU_FOLLOW) - 0.9502) < 0.001);
  assert.ok(DT_MAX > 0 && DT_MAX < 0.1);
});

test("the move cap is a speed, not a per-frame step (issue #63)", () => {
  const radius = 200;
  // At 60 Hz the new cap is exactly the old per-frame share, so the familiar feel is kept.
  assert.ok(Math.abs(moveCap(radius, 1 / 60) - radius * MOVE_SHARE) < 1e-9);
  assert.ok(Math.abs(MOVE_RATE - 60 * MOVE_SHARE) < 1e-9);
  // Twice the frames in a second each move half as far.
  assert.ok(Math.abs(moveCap(radius, 1 / 144) * 144 - moveCap(radius, 1 / 60) * 60) < 1e-6);
  assert.ok(Math.abs(moveCap(radius, 1 / 30) * 30 - moveCap(radius, 1 / 60) * 60) < 1e-6);
});

test("a long sweep takes the same wall-clock time at 30, 60 and 144 Hz (issue #63)", () => {
  const distance = 600; // the sweep the review measured
  const t30 = sweepSeconds(distance, 30);
  const t60 = sweepSeconds(distance, 60);
  const t144 = sweepSeconds(distance, 144);
  // The old per-frame cap took ~0.5 s at 144 Hz, ~0.7 s at 60 Hz, ~1.1 s at 30 Hz.
  assert.ok(Math.abs(t30 - t60) < 0.02, `30 Hz ${t30}s vs 60 Hz ${t60}s`);
  assert.ok(Math.abs(t60 - t144) < 0.02, `60 Hz ${t60}s vs 144 Hz ${t144}s`);
});
