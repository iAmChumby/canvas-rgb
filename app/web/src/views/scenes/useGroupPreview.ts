"use client";

// The per-group preview (#378): `GET /api/v1/preview/group?group=<id>&token=<token>`, a
// WebSocket (canvas-proto `routes::PREVIEW_GROUP`). Its messages are binary frames in the
// effects preview's format: u16 LE width, u16 LE height, then width*height*3 RGB bytes. The
// group always sends 80x50, at most 15 a second.
//
// One stage, one socket. The socket is opened for the group the stage shows and closed when
// the stage goes, when the sheet closes and when the window is hidden. A close with 1013 (a
// newer group preview took over) is not tried again: the stage says the preview stopped and
// offers a key to start it by hand. Any other close is tried again with the light feed's
// backoff; after the last try the stage says the preview stopped.
//
// A group the daemon has not saved yet gets no socket at all: the daemon answers 404 for a
// group it does not hold, so the stage waits for the save to land and opens the socket then.
//
// A socket that opens and says nothing is not waited for for ever. The daemon sends the first
// picture at once (a new selection gets its picture on the engine's first tick), so a socket
// that stays quiet past the wait has nothing to say: the stage says the preview stopped and
// the owner starts it again by hand. A paused engine says nothing by design, so the wait does
// not count while the lights are paused.

import { useCallback, useEffect, useRef, useState } from "react";
import { apiConfig, dropApiConfig } from "@/lib/api";
import { demoGroupFrame } from "@/lib/demoGroupPreview";
import { demoGroupPreviewMode } from "@/lib/demoScenarios";
import { GROUP_PREVIEW_H, GROUP_PREVIEW_W, GROUP_TAKEN_CODE } from "@/lib/groupPreview";
import { isDemo } from "@/lib/host";
import { parseFrame, type Frame } from "@/lib/preview";
import { getSnap } from "@/lib/store";
import { GROUP_PREVIEW_QUERY, PREVIEW_TOKEN_QUERY } from "@/proto/consts";
import { routes } from "@/proto/routes";
import { retryWait } from "@/views/scenes/useLightFeed";

/** How many times a refused socket is tried again before the stage gives up. */
export const GROUP_PREVIEW_RETRIES = 3;

/** The pictures the daemon sends, at most this many a second (`PREVIEW_MAX_FPS`). */
const GROUP_PREVIEW_FPS = 15;

/**
 * How long a socket may say nothing before the stage says the preview stopped.
 *
 * The daemon sends the first picture at once, so this is the wait for a socket that has
 * nothing to say, not the wait for a slow machine. It is the light feed's own first wait
 * (1.2 s), the same number the feed waits before it tries a closed socket again.
 */
export const GROUP_PREVIEW_QUIET_MS = retryWait(0);

export type GroupPreviewState = "connecting" | "live" | "stopped";

export interface GroupPreviewOptions {
  /** The group to show. Null when the stage shows no picture at all (a pattern, or Off). */
  groupId: string | null;
  /** True when the daemon holds this group in its saved scene. A group that is not saved yet
   *  gets no socket: the daemon answers 404 for a group it does not hold. */
  saved: boolean;
}

export interface GroupPreview {
  /** connecting: the socket is opening, or the group's save has not landed. live: a picture has
   *  arrived. stopped: the socket gave up. */
  state: GroupPreviewState;
  /** Paints every picture as it arrives. The returned function stops the painting. */
  onFrame(listener: (frame: Frame) => void): () => void;
  /** Starts the preview again by hand, after it stopped. */
  retry(): void;
}

/** The socket's own life: one group, one socket, until the stage lets it go. */
interface Feed {
  /** Starts it again by hand after it stopped. */
  retry(): void;
  /** Closes the socket and gives the timers back. */
  stop(): void;
}

/**
 * Opens the socket for one group and keeps it until the stage lets it go.
 *
 * The frame is read with the effects preview's own reader (`lib/preview.ts`), so the wire
 * format has one reader in the editor.
 */
function openGroupFeed(
  groupId: string,
  handlers: {
    /** The stage's state changed. */
    onState: (state: GroupPreviewState) => void;
    /** A picture arrived. */
    onFrame: (frame: Frame) => void;
  },
): Feed {
  let stopped = false;
  let socket: WebSocket | null = null;
  // A connect is waiting for the token: the socket is set only after that wait, so the wait
  // itself is claimed. Without the claim a second call in that window would open a second
  // socket and lose the first one.
  let connecting = false;
  let retry: ReturnType<typeof setTimeout> | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;
  // The wait for the first picture: the socket is open and has said nothing yet.
  let quiet: ReturnType<typeof setTimeout> | null = null;
  let failures = 0;

  const visible = () => typeof document === "undefined" || document.visibilityState === "visible";

  /** The lights are paused: the engine sends nothing at all, and that is on purpose. */
  const paused = () => getSnap().status?.power === "paused";

  const close = () => {
    if (retry) {
      clearTimeout(retry);
      retry = null;
    }
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
    if (quiet) {
      clearTimeout(quiet);
      quiet = null;
    }
    if (socket) {
      socket.onclose = null;
      socket.onerror = null;
      socket.onmessage = null;
      socket.close();
      socket = null;
    }
  };

  /**
   * The wait for the first picture.
   *
   * The socket stays open when the wait runs out: a paused engine says nothing by design and
   * says the picture again when it runs, so the stage is told the preview stopped but the
   * socket is not thrown away.
   */
  const armQuiet = () => {
    if (quiet) clearTimeout(quiet);
    quiet = setTimeout(() => {
      quiet = null;
      if (stopped || !socket) return;
      // The lights are paused: the engine says nothing on purpose, so the wait starts again.
      if (paused()) {
        armQuiet();
        return;
      }
      handlers.onState("stopped");
    }, GROUP_PREVIEW_QUIET_MS);
  };

  const schedule = () => {
    if (stopped || retry || !visible()) return;
    if (failures >= GROUP_PREVIEW_RETRIES) {
      handlers.onState("stopped");
      return;
    }
    retry = setTimeout(() => {
      retry = null;
      void connect();
    }, retryWait(failures));
    failures += 1;
  };

  const connect = async (): Promise<void> => {
    if (stopped || socket || connecting || !visible()) return;
    // The claim is taken before the wait for the token and given back on every way out.
    connecting = true;
    let url: string;
    try {
      const cfg = await apiConfig();
      const query = `${GROUP_PREVIEW_QUERY}=${encodeURIComponent(groupId)}&${PREVIEW_TOKEN_QUERY}=${encodeURIComponent(cfg.token)}`;
      url = `${cfg.base_url.replace(/^http/, "ws")}${routes.previewGroup}?${query}`;
    } catch {
      connecting = false;
      schedule();
      return;
    }
    // The wait for the token may have outlived the stage or the window.
    if (stopped || socket || !visible()) {
      connecting = false;
      return;
    }
    // The constructor throws on an address it cannot parse: the claim goes back and the retry
    // runs, or the stage would never connect again.
    let ws: WebSocket;
    try {
      ws = new WebSocket(url);
    } catch {
      connecting = false;
      schedule();
      return;
    }
    ws.binaryType = "arraybuffer";
    ws.onopen = () => {
      failures = 0;
      // The socket is open and has said nothing yet: the wait for the first picture starts.
      armQuiet();
    };
    ws.onmessage = (event: MessageEvent) => {
      if (!(event.data instanceof ArrayBuffer)) return;
      const frame = parseFrame(event.data);
      if (!frame) return;
      // A picture has come: the socket has something to say.
      if (quiet) {
        clearTimeout(quiet);
        quiet = null;
      }
      handlers.onState("live");
      handlers.onFrame(frame);
    };
    ws.onclose = (event: CloseEvent) => {
      socket = null;
      if (quiet) {
        clearTimeout(quiet);
        quiet = null;
      }
      if (stopped) return;
      // Another group preview took over: the stage says the preview stopped, and the owner
      // starts it again by hand. It is not tried again by itself.
      if (event.code === GROUP_TAKEN_CODE) {
        handlers.onState("stopped");
        return;
      }
      // The tray may have handed out a new token: ask again.
      dropApiConfig();
      schedule();
    };
    socket = ws;
    connecting = false;
  };

  /** The demo has no socket: it draws the group's own picture, as the daemon sends it. */
  const startDemo = () => {
    const mode = demoGroupPreviewMode();
    if (mode === "stopped") {
      // The daemon refused the socket: the stage says the preview stopped.
      handlers.onState("stopped");
      return;
    }
    if (mode === "connecting") {
      // The socket is open and has said nothing yet.
      handlers.onState("connecting");
      return;
    }
    const t0 = performance.now();
    const tick = () => {
      const rgb = demoGroupFrame(groupId, (performance.now() - t0) / 1000);
      if (!rgb) return;
      handlers.onState("live");
      handlers.onFrame({ width: GROUP_PREVIEW_W, height: GROUP_PREVIEW_H, rgb });
    };
    tick();
    timer = setInterval(tick, 1000 / GROUP_PREVIEW_FPS);
  };

  const start = () => {
    if (stopped || socket || timer) return;
    if (isDemo()) {
      startDemo();
      return;
    }
    void connect();
  };

  const onVisibility = () => {
    if (visible()) start();
    else {
      // The window went away: the socket goes with it, and the next one starts fresh.
      failures = 0;
      close();
    }
  };

  start();
  document.addEventListener("visibilitychange", onVisibility);

  return {
    retry: () => {
      if (stopped) return;
      if (retry) {
        clearTimeout(retry);
        retry = null;
      }
      failures = 0;
      handlers.onState("connecting");
      // The old socket goes first: a socket that said nothing is still open, and a new one is
      // made for the same group.
      close();
      start();
    },
    stop: () => {
      stopped = true;
      document.removeEventListener("visibilitychange", onVisibility);
      close();
    },
  };
}

/**
 * Holds the group's preview while the stage is mounted.
 *
 * The socket is opened for the group the stage shows, and only for a group the daemon has
 * saved: a group that is still a draft is answered with 404, so the stage waits for the save
 * instead. A picture never causes a render: the frames go to the painter, and only the
 * stage's own state is React's.
 */
export function useGroupPreview({ groupId, saved }: GroupPreviewOptions): GroupPreview {
  const [state, setState] = useState<GroupPreviewState>("connecting");
  const listeners = useRef(new Set<(frame: Frame) => void>());
  const feed = useRef<Feed | null>(null);
  // The state the stage shows, as the feed's own last word. A picture that arrives does not
  // change it, so a frame never causes a render.
  const shown = useRef<GroupPreviewState>("connecting");

  const onState = useCallback((next: GroupPreviewState) => {
    if (shown.current === next) return;
    shown.current = next;
    setState(next);
  }, []);

  const onFrame = useCallback((listener: (frame: Frame) => void) => {
    listeners.current.add(listener);
    return () => {
      listeners.current.delete(listener);
    };
  }, []);

  useEffect(() => {
    // The stage shows another group, or the group is not saved yet: the old socket goes first
    // (the effect's own cleanup), and the stage waits.
    onState("connecting");
    if (!groupId || !saved) return;
    const open = openGroupFeed(groupId, {
      onState,
      onFrame: (frame) => {
        for (const listener of listeners.current) listener(frame);
      },
    });
    feed.current = open;
    return () => {
      open.stop();
      if (feed.current === open) feed.current = null;
    };
  }, [groupId, saved, onState]);

  const retry = useCallback(() => {
    feed.current?.retry();
  }, []);

  return { state, onFrame, retry };
}
