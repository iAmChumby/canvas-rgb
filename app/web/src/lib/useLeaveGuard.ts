"use client";

// The guard an editor with unsaved work registers while it has changes (#474).
//
// Three things are the same for every one of them, and they live here:
//   - the shell asks the guard before it changes page (`lib/navigationGuard.ts`): a rail link, a
//     banner, the tray's own navigate,
//   - the browser's Back lands on an entry of the editor's own, so Back asks instead of
//     leaving, and that entry is taken back when there is nothing left to lose,
//   - the browser is warned when the window is closed.
//
// What the editor does with the answer is its own: the stack builder rewrites the URL of the
// entry it lands on, and the pattern editor's Cancel goes to the effects page.

import { useCallback, useEffect, useRef } from "react";
import { cancelNavigation, setNavigationGuard } from "@/lib/navigationGuard";

/** Why the prompt opened: the Cancel key, an in-app page change, or the browser's Back. */
export type LeaveReason = "close" | "navigation" | "back";

export function useLeaveGuard({
  dirty,
  sentinel,
  onAsk,
  onDrop,
  isLeaving,
}: {
  /** True while the editor has work that leaving would lose. */
  dirty: boolean;
  /** The `history.state` key that marks the guard's own entry. */
  sentinel: string;
  /** The prompt opens. */
  onAsk: (reason: LeaveReason) => void;
  /** The guard's entry is taken back, because there is nothing left to lose. */
  onDrop?: () => void;
  /** True while a leave is under way: the guard's entry stays then. */
  isLeaving?: () => boolean;
}): {
  /**
   * Lets the guard's own listener ignore the next history move: call it before a move the
   * editor makes on its own, so the guard does not catch its own leave.
   */
  allowBack: () => void;
  /**
   * Takes the guard's entry off the history. `done` runs once the history has moved; without
   * it the entry is simply taken back.
   */
  dropSentinel: (done?: () => void) => void;
} {
  // The helpers are read through refs: only a change to `dirty` may register or drop the guard,
  // or the history entry would be pushed again on every render.
  const ask = useRef(onAsk);
  const drop = useRef(onDrop);
  const leaving = useRef(isLeaving);
  useEffect(() => {
    ask.current = onAsk;
    drop.current = onDrop;
    leaving.current = isLeaving;
  });

  /** The guard's entry is on the history. */
  const pushed = useRef(false);
  /** The editor is leaving on its own: the guard must not catch that move. */
  const free = useRef(false);

  const allowBack = useCallback(() => {
    free.current = true;
  }, []);

  const dropSentinel = useCallback((done?: () => void) => {
    if (!pushed.current) {
      done?.();
      return;
    }
    pushed.current = false;
    if (!done) {
      window.history.back();
      return;
    }
    const landed = () => {
      window.removeEventListener("popstate", landed);
      done();
    };
    window.addEventListener("popstate", landed);
    window.history.back();
  }, []);

  useEffect(() => {
    if (!dirty) {
      setNavigationGuard(null);
      if (pushed.current) {
        if (leaving.current?.() ?? false) {
          // A leave that is under way navigates itself; taking the entry back first would move
          // the history one extra time.
          pushed.current = false;
        } else if (drop.current) {
          drop.current();
          pushed.current = false;
        } else {
          dropSentinel();
        }
      }
      return;
    }
    setNavigationGuard(() => {
      ask.current?.("navigation");
      return false;
    });
    const onPop = () => {
      if (free.current) return;
      // The Back landed on the entry before the guard's own: put the guard's entry back, so the
      // page stays where it is and the prompt can ask.
      window.history.pushState({ ...(window.history.state ?? {}), [sentinel]: true }, "", window.location.href);
      pushed.current = true;
      ask.current?.("back");
    };
    window.history.pushState({ ...(window.history.state ?? {}), [sentinel]: true }, "", window.location.href);
    pushed.current = true;
    window.addEventListener("popstate", onPop);
    return () => {
      window.removeEventListener("popstate", onPop);
      setNavigationGuard(null);
    };
  }, [dirty, sentinel, dropSentinel]);

  // The browser's own navigation: it asks in its words.
  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

  // An editor that goes away without the prompt being answered leaves no held page change. A
  // leave that is under way carries the page change out itself, so it is not dropped here.
  useEffect(
    () => () => {
      setNavigationGuard(null);
      if (!(leaving.current?.() ?? false)) cancelNavigation();
    },
    [],
  );

  return { allowBack, dropSentinel };
}
