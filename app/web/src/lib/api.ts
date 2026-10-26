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

export async function apiConfig(): Promise<ApiConfig> {
  if (!cachedConfig) cachedConfig = await fetchApiConfig();
  return cachedConfig;
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
    throw new ApiFailure(0, "unreachable", "The daemon is not answering on 127.0.0.1:9430.");
  }

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
