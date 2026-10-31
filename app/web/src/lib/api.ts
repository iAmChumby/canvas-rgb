// HTTP client for the daemon's loopback API (crates/canvas-proto/src/api.rs).
// Every request carries `Authorization: Bearer <token>` (G9); the token is read at runtime
// from the Tauri host and never enters the bundle. In a plain browser the call goes to the
// demo backend instead (see demo.ts), with the same paths and the same error shapes.

import type { ApiConfig } from "@/proto/ApiConfig";
import type { ApiError } from "@/proto/ApiError";
import { fetchApiConfig, isDemo } from "@/lib/host";
import { demoRequest } from "@/lib/demo";
import { ApiFailure, messageOf } from "@/lib/errors";

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

/** Forgets the token. The next request fetches it again, which runs the tray's peer check again. */
export function dropApiConfig(): void {
  cachedConfig = null;
  pendingConfig = null;
}

export type Method = "GET" | "PUT" | "PATCH" | "POST" | "DELETE";

/** Sends one request to the daemon and returns the raw response, or throws ApiFailure. */
async function send(method: Method, path: string, body?: unknown): Promise<Response> {
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
    });
  } catch {
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
    throw new ApiFailure(res.status, code, message);
  }
  return res;
}

export async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  if (isDemo()) {
    return (await demoRequest(method, path, body)) as T;
  }
  const res = await send(method, path, body);
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  if (text.length === 0) return undefined as T;
  return JSON.parse(text) as T;
}

/** A GET whose answer is binary (the effect thumbnails). */
export async function requestBinary(path: string): Promise<ArrayBuffer> {
  if (isDemo()) {
    return (await demoRequest("GET", path, undefined)) as ArrayBuffer;
  }
  const res = await send("GET", path);
  return res.arrayBuffer();
}
