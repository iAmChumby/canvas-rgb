// The world a preview is proved against (#579): a tray host that hands out the real daemon's
// address, and a counter on every way out of the page and every storage write.
//
// Import it through `vi.hoisted`, before the app's modules: the preview flag is read at load.

export interface World {
  /** Every address asked of `fetch`. */
  fetched: string[];
  /** Every socket's address. */
  sockets: string[];
  /** Every command sent to the tray. */
  invoked: string[];
  /** Every storage write, as `store method key`. */
  writes: string[];
}

export const DAEMON = "127.0.0.1:9430";

export function makeWorld(address: string): World {
  const world: World = { fetched: [], sockets: [], invoked: [], writes: [] };
  window.history.replaceState({}, "", address);

  for (const method of ["setItem", "removeItem", "clear"] as const) {
    const real = Storage.prototype[method] as (...args: string[]) => void;
    Storage.prototype[method] = function (this: Storage, ...args: string[]) {
      const store = this === window.sessionStorage ? "session" : "local";
      world.writes.push(`${store} ${method} ${args[0] ?? ""}`);
      real.apply(this, args);
    };
  }

  globalThis.fetch = (async (input: RequestInfo | URL) => {
    world.fetched.push(input instanceof Request ? input.url : String(input));
    throw new TypeError("no network in this test");
  }) as typeof fetch;

  globalThis.WebSocket = class {
    binaryType = "blob";
    onopen: unknown = null;
    onmessage: unknown = null;
    onerror: unknown = null;
    onclose: unknown = null;
    readyState = 0;
    constructor(url: string) {
      world.sockets.push(url);
    }
    send(): void {}
    close(): void {}
  } as unknown as typeof WebSocket;

  (window as unknown as { __TAURI__?: unknown }).__TAURI__ = {
    core: {
      invoke: async (cmd: string) => {
        world.invoked.push(cmd);
        return { base_url: `http://${DAEMON}`, token: "test-token" };
      },
    },
    event: { listen: async () => () => undefined },
  };

  return world;
}

export function stubBrowser(): void {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  globalThis.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
}
