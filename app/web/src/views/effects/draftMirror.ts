// The builder's newest draft, for the code that runs after an await (#332).
//
// A draft change reaches two places: the state React renders, and this mirror. The builder's
// async edits read the mirror, because a promise that resolves later cannot see the state it was
// rendered with.
//
// React commits a state in its own time. Under load, a commit can arrive after a later change has
// already been written into the mirror: the builder's first commit (its empty draft, from a
// passive effect flushed late) landed after a starter had written its three layers, put the mirror
// back, and the next queued starter composed on an empty stack. The reverse also happens: a load
// or a save can reset the draft *after* an edit's write, and that reset is the newer truth.
//
// So every draft carries the version of the change that produced it (`useStackHistory`), and the
// mirror only moves forward: a write or a commit older than what it holds is ignored. Versions are
// compared, not arrival times, so neither order can lose the newer draft.

import type { Stack } from "@/proto/Stack";

export class DraftMirror {
  /** The newest draft, in the shape every reader expects of a ref (`mirror.current`). */
  current: Stack;
  /** The version of the change the mirror holds. */
  private at: number;

  constructor(draft: Stack, at = 0) {
    this.current = draft;
    this.at = at;
  }

  /** The edit code wrote this draft, as change `version`: it is the newest from now on. */
  write(next: Stack, version: number): void {
    if (version < this.at) return;
    this.current = next;
    this.at = version;
  }

  /** React committed a draft, as change `version`: follow it unless the mirror holds a newer one. */
  commit(committed: Stack, version: number): void {
    if (version < this.at) return;
    this.current = committed;
    this.at = version;
  }
}
