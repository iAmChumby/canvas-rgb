// The arithmetic the chain flow is built on: how a header splits into parts, what a row's LED
// count does to its neighbour, and the runs the daemon is told to light.
//
// The split must agree with the daemon's own (`crates/canvas-daemon/src/chain_plan.rs`,
// `even_split`): the remainder goes to the first parts, so 96 into 7 is 14 14 14 14 14 13 13.
// Nothing here reads a clock or the network; the hook does the sending.

import { PART_COLOURS } from "@/views/chain/partColours";

/** One coloured run, by LED index inside the zone (canvas-proto `SetupRun`). */
export interface Run {
  start: number;
  count: number;
  color: string;
}

/**
 * `len` LEDs split into `count` parts as evenly as they go: the first `len % count` parts get one
 * LED more. Refuses a count of zero and a count the length cannot hold, as the daemon does.
 */
export function evenSplit(len: number, count: number): number[] {
  if (!Number.isInteger(len) || !Number.isInteger(count)) {
    throw new Error(`a chain of ${len} LEDs cannot be split into ${count} parts`);
  }
  if (count < 1) throw new Error("a chain has at least one part");
  if (count > len) throw new Error(`${count} parts do not fit in ${len} LEDs`);
  const base = Math.floor(len / count);
  const remainder = len % count;
  return Array.from({ length: count }, (_, i) => base + (i < remainder ? 1 : 0));
}

/**
 * A row's length changed to `newLen`. The next row takes the difference, so the chain keeps its
 * total; on the last row there is no next row and the total changes. No row goes below one LED:
 * the move stops where its neighbour's last LED is.
 */
export function moveBoundary(lengths: number[], row: number, newLen: number): number[] {
  if (!Number.isInteger(row) || row < 0 || row >= lengths.length) {
    throw new Error(`there is no row ${row} in a chain of ${lengths.length} parts`);
  }
  const next = lengths.slice();
  const isLast = row === lengths.length - 1;
  if (isLast) {
    next[row] = Math.max(1, Math.round(newLen));
    return next;
  }
  const here = lengths[row] ?? 1;
  const below = lengths[row + 1] ?? 1;
  // The two rows hold a fixed number of LEDs between them; the neighbour keeps at least one.
  const most = here + below - 1;
  const wanted = Math.min(Math.max(1, Math.round(newLen)), Math.max(1, most));
  next[row] = wanted;
  next[row + 1] = here + below - wanted;
  return next;
}

/**
 * The runs the daemon lights for a chain: the parts in order, each starting where the one before
 * it ended, each in its colour from `PART_COLOURS` (a chain longer than the colour set starts it
 * again).
 */
export function runsFor(lengths: number[]): Run[] {
  let start = 0;
  return lengths.map((count, i) => {
    const run: Run = {
      start,
      count,
      color: PART_COLOURS[i % PART_COLOURS.length] ?? PART_COLOURS[0] ?? "#ffffff",
    };
    start += count;
    return run;
  });
}
