/**
 * The hero glow's motion math (issues #15 and #63).
 *
 * Pure functions and constants only: no DOM, no React, no framework. That is what lets the
 * site's regression tests run under the built-in Node test runner with no new dependency,
 * and it keeps the rule in one place for both the component and its tests.
 */

/** The most elapsed time one frame is allowed to ease over, in seconds. */
export const DT_MAX = 0.05;
/** The follow time constant, in seconds (a short ease, on time, not on frames). */
export const TAU_FOLLOW = 0.075;
/** The settle-back time constant, in seconds. */
export const TAU_SETTLE = 0.45;
/**
 * The furthest the light may travel in one 60 Hz frame, as a share of its own radius.
 * Kept as the familiar per-frame share; `MOVE_RATE` is the same limit expressed per second.
 */
export const MOVE_SHARE = 1 / 16;
/**
 * The move cap per second. The cap is scaled by the frame's elapsed time, so a long sweep
 * takes the same wall-clock time at 30, 60 and 144 Hz (issue #63) instead of taking longer
 * the lower the refresh rate.
 */
export const MOVE_RATE = 60 * MOVE_SHARE;

/** The share of the gap to the pointer a frame closes. */
export const easeK = (dt: number, tau: number): number => 1 - Math.exp(-dt / tau);

/**
 * The furthest the light may travel in one frame of length `dt`, in the same units as
 * `radius`. `dt` is the (capped) elapsed time, so the cap is a speed limit, not a per-frame
 * step: twice as many frames in a second each move half as far.
 */
export const moveCap = (radius: number, dt: number): number => radius * MOVE_RATE * dt;
