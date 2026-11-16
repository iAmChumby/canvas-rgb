// The frame loop, stubbed, for the tests that drive the pad by hand.
//
// The pad is read in a `requestAnimationFrame` loop (#375). These tests step that loop
// themselves: the test says when a frame runs. No test here waits for the clock, so the verdict
// never depends on how fast the machine is (#59).

import { act } from "@testing-library/react";

/** The stubbed loop: it counts the frames that are pending and the ones that were called off. */
export class PadFrames {
  private next = 1;
  private readonly pending = new Map<number, FrameRequestCallback>();
  /** The frames that were called off, in the order they were called off. */
  readonly cancelled: number[] = [];

  /** Takes over the window's frame loop. */
  install() {
    window.requestAnimationFrame = (cb: FrameRequestCallback) => {
      const id = this.next++;
      this.pending.set(id, cb);
      return id;
    };
    window.cancelAnimationFrame = (id: number) => {
      this.cancelled.push(id);
      this.pending.delete(id);
    };
  }

  /** How many frames are waiting to run. */
  get pendingCount(): number {
    return this.pending.size;
  }

  /** Runs every frame that is due, the way the browser would. */
  step() {
    const due = [...this.pending.values()];
    this.pending.clear();
    act(() => {
      for (const cb of due) cb(0);
    });
  }
}
