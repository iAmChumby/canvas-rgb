// A model of the session history for tests that assert the whole list (#474).
//
// jsdom keeps a session history but gives no way to read it, so a test that wants to say "the
// history is [page, effects]" cannot. The tracker wraps `pushState`, `replaceState` and the
// traversals on `window.history` and keeps the list itself, by the rules a browser follows: a push
// drops every forward entry, a replace rewrites the current one, a traversal moves the index, and
// a traversal past either end leaves the document (the browser's own answer; jsdom would move to
// an entry an earlier test made). The model moves at once and jsdom a tick later, so a test waits
// for `location` to agree before it asserts.

/** One entry of the list: the address and the state it was pushed with. */
export interface TrackedEntry {
  url: string;
  state: unknown;
}

export interface HistoryTracker {
  entries: TrackedEntry[];
  /** The entry the document is on. */
  index: number;
  /** True once a traversal asked for an entry before the first or after the last. */
  left: boolean;
  /** The addresses, each followed by ` (<flag>)` when its state carries one of `flags`. */
  urls: (flags?: string[]) => string[];
  stop: () => void;
}

const relative = (url: string | URL | null | undefined): string => {
  const target = new URL(String(url ?? window.location.href), window.location.href);
  return target.pathname + target.search;
};

/** Starts tracking from the entry the document is on now. */
export function trackHistory(): HistoryTracker {
  const history = window.history;
  const original = {
    pushState: history.pushState,
    replaceState: history.replaceState,
    go: history.go,
    back: history.back,
    forward: history.forward,
  };
  const tracker: HistoryTracker = {
    entries: [{ url: relative(null), state: history.state }],
    index: 0,
    left: false,
    urls: (flags = ["canvasRgbPatternGuard", "canvasRgbStackGuard"]) =>
      tracker.entries.map((entry) => {
        const state = (entry.state ?? {}) as Record<string, unknown>;
        const marked = flags.filter((flag) => state[flag] !== undefined);
        return marked.length > 0 ? `${entry.url} (guard)` : entry.url;
      }),
    stop: () => {
      history.pushState = original.pushState;
      history.replaceState = original.replaceState;
      history.go = original.go;
      history.back = original.back;
      history.forward = original.forward;
    },
  };
  const traverse = (delta: number) => {
    const target = tracker.index + delta;
    if (target < 0 || target >= tracker.entries.length) {
      tracker.left = true;
      return;
    }
    tracker.index = target;
    original.go.call(history, delta);
  };
  history.pushState = (state: unknown, title: string, url?: string | URL | null) => {
    tracker.entries.splice(tracker.index + 1);
    tracker.entries.push({ url: relative(url), state });
    tracker.index = tracker.entries.length - 1;
    original.pushState.call(history, state, title, url);
  };
  history.replaceState = (state: unknown, title: string, url?: string | URL | null) => {
    tracker.entries[tracker.index] = { url: relative(url), state };
    original.replaceState.call(history, state, title, url);
  };
  history.go = (delta?: number) => {
    if (delta === undefined || delta === 0) original.go.call(history, delta);
    else traverse(delta);
  };
  history.back = () => traverse(-1);
  history.forward = () => traverse(1);
  return tracker;
}
