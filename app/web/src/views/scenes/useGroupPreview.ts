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
// A socket that fails before it opened while the save is still in flight waits for it too,
// and never says the preview stopped.

import { useCallback, useEffect, useRef, useState } from "react";
import { apiConfig, dropApiConfig } from "@/lib/api";
import { demoGroupFrame } from "@/lib/demoGroupPreview";
import { GROUP_PREVIEW_H, GROUP_PREVIEW_W, GROUP_TAKEN_CODE } from "@/lib/groupPreview";
import { isDemo } from "@/lib/host";
import { parseFrame, type Frame } from "@/lib/preview";
import { GROUP_PREVIEW_QUERY, PREVIEW_TOKEN_QUERY } from "@/proto/consts";
import { routes } from "@/proto/routes";
import { retryWait } from "@/views/scenes/useLightFeed";

/** How many times a refused socket is tried again before the stage gives up. */
export const GROUP_PREVIEW_RETRIES = 3;

/** The pictures the daemon sends, at most this many a second (`PREVIEW_MAX_FPS`). */
const GROUP_PREVIEW_FPS = 15;

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
    /** True while the daemon holds this group in its saved scene. */
    isSaved: () => boolean;
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
  let failures = 0;

  const visible = () => typeof document === "undefined" || document.visibilityState === "visible";

  const close = () => {
    if (retry) {
      clearTimeout(retry);
      retry = null;
    }
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
    if (socket) {
      socket.onclose = null;
      socket.onerror = null;
      socket.onmessage = null;
      socket.close();
      socket = null;
    }
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
    };
    ws.onmessage = (event: MessageEvent) => {
      if (!(event.data instanceof ArrayBuffer)) return;
      const frame = parseFrame(event.data);
      if (!frame) return;
      handlers.onState("live");
      handlers.onFrame(frame);
    };
    ws.onclose = (event: CloseEvent) => {
      socket = null;
      if (stopped) return;
      // Another group preview took over: the stage says the preview stopped, and the owner
      // starts it again by hand. It is not tried again by itself.
      if (event.code === GROUP_TAKEN_CODE) {
        handlers.onState("stopped");
        return;
      }
      // The group is not in the daemon's saved scene yet: the save has not landed. The stage
      // waits for it instead of saying the preview stopped.
      if (!handlers.isSaved()) {
        handlers.onState("connecting");
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
  // Whether the daemon holds this group in its saved scene, read at the moment it matters.
  const isSaved = useRef(saved);
  isSaved.current = saved;

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
      isSaved: () => isSaved.current,
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
