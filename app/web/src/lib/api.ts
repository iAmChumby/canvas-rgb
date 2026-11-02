// HTTP client for the daemon's loopback API (crates/canvas-proto/src/api.rs).
// Every request carries `Authorization: Bearer <token>` (G9); the token is read at runtime
// from the Tauri host and never enters the bundle. In a plain browser the call goes to the
// demo backend instead (see demo.ts), with the same paths and the same error shapes.

import type { ApiConfig } from "@/proto/ApiConfig";
import type { ApiError } from "@/proto/ApiError";
import { fetchApiConfig, isDemo } from "@/lib/host";
import { demoRequest } from "@/lib/demo";
import { ApiFailure, messageOf } from "@/lib/errors";
import { routes } from "@/proto/routes";

export { ApiFailure, isUnreachable, messageOf } from "@/lib/errors";

let cachedConfig: ApiConfig | null = null;
let pendingConfig: Promise<ApiConfig> | null = null;

/**
 * The address and token the tray hands out. Every fetch re-runs the tray's check that the 9430
 * listener is canvas-rgbd.exe under this user, so the cache lives only while requests succeed:
 * `dropApiConfig` clears it on a network failure and on a 401, and the next request asks the
 * tray again. Requests that start together share one lookup.
 */
export async function apiConfig(): Promise<ApiConfig> {
  if (cachedConfig) return cachedConfig;
  if (!pendingConfig) {
    const lookup: Promise<ApiConfig> = fetchApiConfig().then((c) => {
      if (pendingConfig === lookup) cachedConfig = c;
      return c;
    });
    pendingConfig = lookup;
    const clear = () => {
      if (pendingConfig === lookup) pendingConfig = null;
    };
    lookup.then(clear, clear);
  }
  return pendingConfig as Promise<ApiConfig>;
}

/**
 * Lets go of a lookup that is still running. A lookup that hangs would otherwise be shared by
 * every later request, and each of them would wait out its deadline on the same dead promise.
 * The next request asks the tray again. A lookup that finishes late is dropped too: only the
 * lookup that is current may fill the cache.
 */
function abandonPendingConfig(): void {
  pendingConfig = null;
}

/** Forgets the token. The next request fetches it again, which runs the tray's peer check again. */
export function dropApiConfig(): void {
  cachedConfig = null;
  pendingConfig = null;
}

export type Method = "GET" | "PUT" | "PATCH" | "POST" | "DELETE";

/**
 * The deadline of a request that answers in a moment. A request that never answers would
 * leave the area pending for good: no error, no button, and nothing to try again. The daemon
 * answers in well under a second, so this is only the ceiling for a daemon that has stopped
 * mid-call.
 */
export const REQUEST_TIMEOUT_MS = 10_000;

/**
 * The deadline of the peripherals: the routes that open a device and read it.
 *
 * The read of a peripheral is the slow one: the mouse may be asleep, and the adapter then
 * makes two passes over its five reports with a wait for the mouse to wake in between. The
 * worst case comes from the adapter's own constants
 * (`crates/canvas-adapters/src/attack_shark_x6/mod.rs`, next to `Pace`):
 *
 * - one pass, every report refused: 5 reports x 2 attempts (`GRANT_ATTEMPTS`) x 250 ms
 *   (`grant_wait`) = 2.5 s, plus the pause between the reports, 4 x 500 ms (`report_pause`)
 *   = 2.0 s -> 4.5 s;
 * - the wait for the mouse to wake: up to 5 s (`wake_wait`);
 * - a second pass, the mouse awake and the grant given at once: 5 x 250 ms = 1.25 s, plus
 *   4 x 500 ms = 2.0 s -> 3.25 s.
 *
 * 4.5 s + 5 s + 3.25 s = 12.75 s. The route also enumerates the USB bus and opens the device
 * before it reads it, so the deadline is twice the worst case: 30 s.
 */
export const PERIPHERAL_TIMEOUT_MS = 30_000;

/**
 * The deadline of the effect import: the file is parsed (the effect's script runs), written
 * and the whole effects folder is parsed again. A library of effects takes a while.
 */
export const EFFECT_IMPORT_TIMEOUT_MS = 60_000;

/**
 * The deadline of a draft preview: the daemon parses the draft and renders its 24-frame loop on
 * a blocking thread with the renderer's own 30 s budget (`RENDER_BUDGET`,
 * `crates/canvas-daemon/src/thumbnail.rs`). A draft whose script never returns is stopped at
 * that budget; the request then still answers, so the deadline only gives up on a daemon that
 * stopped mid-render. Five seconds of margin cover handing the answer back.
 */
export const EFFECT_PREVIEW_TIMEOUT_MS = 35_000;

/**
 * The deadline of the config export and import: the whole configuration moves, and the export
 * asks the fan service over the pipe before it answers.
 */
export const CONFIG_TRANSFER_TIMEOUT_MS = 60_000;

/**
 * The deadline of the device rescan: the USB bus is looked at again, from the start.
 */
export const DEVICE_RESCAN_TIMEOUT_MS = 30_000;

/**
 * The deadline of the resize of a zone (`PATCH /devices/{id}/zones/{zone}`): the daemon takes
 * the OpenRGB adapter's lock and waits on OpenRGB's replies while it holds it. The handler
 * (`crates/canvas-daemon/src/api/devices.rs`) stops at its first failed request, so a stuck
 * OpenRGB costs one reply timeout, not one per request. The constant is `REPLY_TIMEOUT`,
 * 5 s (`crates/canvas-adapters/src/openrgb.rs`). The handler takes the lock twice, and each
 * time it can wait out one stalled reply of whoever holds it, then waits on its own:
 *
 * - lock 1, held by a call that is stalled on a reply: 5 s;
 * - `set_zone_size` (the resize, then an enumeration of the controllers), one stalled
 *   reply: 5 s;
 * - lock 2, held by a call that is stalled on a reply: 5 s;
 * - `discover` (an enumeration again), one stalled reply: 5 s.
 *
 * 4 x 5 s = 20 s. A resize that landed must not be reported as a timeout, so the deadline
 * adds half again as margin: 30 s. The deadline is six reply timeouts long (30 s = 6 x
 * `REPLY_TIMEOUT`): four of them are the stalls above, and two of them are the margin.
 *
 * What the 30 s covers is the handler's own requests, with one stall per stage. What it does
 * not cover is another holder of the same lock: the supervisor's reconnect
 * (`crates/canvas-daemon/src/openrgb_supervisor.rs`, `connect_and_prepare`) takes the lock for
 * its TCP connect (no timeout of its own, `openrgb.rs`, `connect`), the handshake, an
 * enumeration of the controllers, the re-apply of every saved zone (each one enumerates them
 * again) and, when any zone is saved, one more enumeration. An enumeration is one request plus
 * one per controller, and each request can wait `REPLY_TIMEOUT`, so on a slow OpenRGB that
 * reconnect can hold the lock past 30 s. A resize that waits for such a reconnect is given up
 * on by the editor, and the daemon may still carry it out.
 */
export const ZONE_RESIZE_TIMEOUT_MS = 30_000;

/**
 * The deadline of one route. The default is short; the routes that wait on a device, or that
 * move a lot of work, get the time their worst case needs. The route is matched on its path,
 * so a request to the same route always gets the same deadline.
 *
 * Every route that awaits an adapter has a line here or a reason it keeps the default:
 *
 * - the peripherals, the zone resize, the effect import, the config transfer and the rescan
 *   have named deadlines above;
 * - `DELETE /displays/{id}` waits for the lock of the display adapter, which the display task
 *   holds for one pass. A pass is one connect (CHIONE: 500 ms `QUERY_WAIT` + up to 1000 ms
 *   `QUERY_REPLY_TIMEOUT_MS` = 1.5 s, `crates/canvas-adapters/src/chione.rs`) and then one
 *   write and ack read per display (`ACK_TIMEOUT_MS`, 100 ms each); its own disconnect is
 *   immediate. About 1.6 s for the one display the machine has, so the default 10 s covers it;
 * - `GET /diag/readback` waits on OpenRGB for each device, but the editor never calls it.
 *
 * The other routes answer from memory or from a file and wait on no device.
 */
export function deadlineFor(path: string): number {
  // The peripherals: the enumeration opens the USB devices, and the read of one waits for a
  // sleeping mouse to wake.
  if (path.startsWith(routes.peripherals)) return PERIPHERAL_TIMEOUT_MS;
  // The effect import: the file is parsed, written and the folder is parsed again.
  if (path.startsWith(routes.effectsImport)) return EFFECT_IMPORT_TIMEOUT_MS;
  // The draft preview: one render of the loop, inside the renderer's 30 s budget.
  if (path === routes.effectPreview) return EFFECT_PREVIEW_TIMEOUT_MS;
  // The config export and import: the whole configuration moves.
  if (path.startsWith(routes.configExport) || path.startsWith(routes.configImport)) {
    return CONFIG_TRANSFER_TIMEOUT_MS;
  }
  // The zone resize: the daemon waits on OpenRGB while it holds the adapter.
  if (path.startsWith(routes.devices) && path.includes("/zones/")) return ZONE_RESIZE_TIMEOUT_MS;
  // The rescan: the USB bus is looked at again.
  if (path.startsWith(routes.devices) && path.includes("rescan")) return DEVICE_RESCAN_TIMEOUT_MS;
  return REQUEST_TIMEOUT_MS;
}

/**
 * Sends one request to the daemon and returns the body it read, or throws ApiFailure.
 *
 * The deadline covers the whole request: the tray's lookup of the address and the token, the
 * call itself, and the body that comes with it. A request that hangs anywhere in that chain
 * is given up on, so the area is never left pending with no error and no way to try again.
 */
async function send<T>(
  method: Method,
  path: string,
  body: unknown | undefined,
  read: (res: Response) => Promise<T>,
): Promise<T> {
  const budget = deadlineFor(path);
  const controller = new AbortController();
  // The deadline: it ends the request, and it is what the frame says when it passes.
  let stopTimer: () => void = () => {};
  const deadline = new Promise<never>((_resolve, reject) => {
    const timer = setTimeout(() => {
      // If the deadline passed during the tray's lookup, that lookup is dead for every request
      // after this one too: the next request asks the tray again.
      abandonPendingConfig();
      controller.abort();
      reject(
        new ApiFailure(
          0,
          "timeout",
          `canvas-rgb did not answer within ${Math.round(budget / 1000)} seconds. It may be stopped, or busy.`,
        ),
      );
    }, budget);
    stopTimer = () => clearTimeout(timer);
  });

  try {
    const work = (async () => {
      let config: ApiConfig;
      try {
        config = await apiConfig();
      } catch (e) {
        throw new ApiFailure(0, "unreachable", `The tray app did not provide the API address: ${messageOf(e)}`);
      }

      const headers: Record<string, string> = { Authorization: `Bearer ${config.token}` };
      if (body !== undefined) headers["Content-Type"] = "application/json";

      let res: Response;
      try {
        res = await fetch(config.base_url + path, {
          method,
          headers,
          body: body === undefined ? undefined : JSON.stringify(body),
          cache: "no-store",
          signal: controller.signal,
        });
      } catch {
        if (controller.signal.aborted) {
          // The deadline passed. The daemon may be busy or gone: either way the editor waits no
          // longer for this answer.
          throw new ApiFailure(
            0,
            "timeout",
            `canvas-rgb did not answer within ${Math.round(budget / 1000)} seconds. It may be stopped, or busy.`,
          );
        }
        // The listener may be gone, or another process may be about to take its place: the token is
        // not trusted again until the tray has checked the listener once more.
        dropApiConfig();
        // A cross-origin request that fails like this is either a service that is down or a reply the
        // browser was not allowed to read. Say so, instead of blaming the network.
        throw new ApiFailure(
          0,
          "unreachable",
          "canvas-rgb did not answer. It may be stopped, or the browser was not allowed to read its reply.",
        );
      }

      if (res.status === 401) dropApiConfig(); // the token changed (a purge and reinstall): ask again

      if (!res.ok) {
        let parsed: Partial<ApiError> = {};
        try {
          parsed = (await res.json()) as Partial<ApiError>;
        } catch {
          parsed = {};
        }
        const code = typeof parsed.error === "string" ? parsed.error : "http";
        const message =
          typeof parsed.message === "string" && parsed.message.length > 0
            ? parsed.message
            : `HTTP ${res.status}`;
        // The preview route says how long to wait while both render slots are busy. Seconds
        // in the header; milliseconds everywhere in the editor.
        const retry = Number(res.headers.get("retry-after"));
        const retryAfterMs = Number.isFinite(retry) && retry > 0 ? retry * 1000 : undefined;
        throw new ApiFailure(res.status, code, message, retryAfterMs);
      }
      // The body is read under the same deadline: a body that never finishes is a timeout too.
      return await read(res);
    })();
    return await Promise.race([work, deadline]);
  } finally {
    stopTimer();
  }
}

export async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  if (isDemo()) {
    return (await demoRequest(method, path, body)) as T;
  }
  return send(method, path, body, async (res) => {
    if (res.status === 204) return undefined as T;
    const text = await res.text();
    if (text.length === 0) return undefined as T;
    return JSON.parse(text) as T;
  });
}

/** A GET whose answer is binary (the effect thumbnails). */
export async function requestBinary(path: string): Promise<ArrayBuffer> {
  if (isDemo()) {
    return (await demoRequest("GET", path, undefined)) as ArrayBuffer;
  }
  return send("GET", path, undefined, (res) => res.arrayBuffer());
}
