// The one pre-filled Report issue URL (#575, R7, P1). One string drives the preview, Copy report
// and the link; the link opens only on the person's click. The cap is on the WHOLE encoded URL,
// measured in encoded characters, because GitHub counts the address it receives.

import type { AboutReport } from "@/proto/AboutReport";
import { scrubDeviceIds } from "@/lib/errors";

/** Total encoded URL budget, measured against GitHub's 414 boundary. */
export const REPORT_URL_MAX = 6000;

const ISSUE_NEW = "https://github.com/iAmChumby/canvas-rgb/issues/new";
const TEMPLATE = "bug_report.yml";

export interface ReportPrefill {
  /** The exact address the button opens. */
  url: string;
  /** The exact `what-happened` text carried by that address, shown in the preview. */
  whatHappened: string;
  /** True when the hardware field was dropped to fit the budget. */
  droppedHardware: boolean;
  /** True when `what-happened` was cut to fit the budget. */
  truncated: boolean;
  /** The full text to copy when the URL could not carry all of it. */
  clipboardText: string;
}

/** A lone surrogate would make `encodeURIComponent` throw; replace it before measuring. */
function safeText(value: string): string {
  let out = "";
  for (const ch of value) {
    try {
      encodeURIComponent(ch);
      out += ch;
    } catch {
      out += "\uFFFD";
    }
  }
  return out;
}

/** Encoded length of one value as the URL will carry it. */
function encodedLength(value: string): number {
  return encodeURIComponent(value).length;
}

/** Cuts on code-point boundaries, not UTF-16 units, so an emoji is never split in half. */
function cutToEncoded(value: string, budget: number): string {
  let out = "";
  let used = 0;
  for (const ch of value) {
    const width = encodedLength(ch);
    if (used + width > budget) break;
    out += ch;
    used += width;
  }
  return out;
}

/**
 * Builds the pre-filled address. `errorTitle` must be a daemon or tray message that was scrubbed
 * before it reached the editor; a local `Error` detail is never passed here (R6). The builder
 * never throws and the returned address is never over `REPORT_URL_MAX` (P1).
 */
export function buildReportUrl(report: AboutReport, errorTitle?: string | null): ReportPrefill {
  const what = errorTitle ? `Error shown: ${safeText(scrubDeviceIds(errorTitle))}\n\n` : "";
  const fields: Record<string, string> = {
    template: TEMPLATE,
    version: safeText(report.version ?? ""),
    windows: safeText(report.windows ?? ""),
    hardware: safeText(report.hardware ?? ""),
    "other-rgb-apps": safeText(report.other_rgb_apps ?? ""),
    "what-happened": what,
  };
  const fullClipboard = `${what}\n\nVersion: ${fields.version}\nWindows: ${fields.windows}\nHardware: ${fields.hardware}`;

  const assemble = (): string => {
    const query = Object.entries(fields)
      .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(value)}`)
      .join("&");
    return `${ISSUE_NEW}?${query}`;
  };

  let droppedHardware = false;
  let truncated = false;
  if (assemble().length > REPORT_URL_MAX) {
    delete fields.hardware;
    droppedHardware = true;
  }
  // Cut the optional detail first, then the error text, then the two short facts if an unusual
  // daemon report still does not fit. Each cut is measured on the encoded address.
  for (const key of ["other-rgb-apps", "what-happened", "windows", "version"]) {
    if (assemble().length <= REPORT_URL_MAX) break;
    const current = fields[key] ?? "";
    const marker = key === "what-happened" ? " (truncated)" : "";
    const overhead = assemble().length - encodedLength(current);
    const room = Math.max(0, REPORT_URL_MAX - overhead - encodedLength(marker));
    fields[key] = cutToEncoded(current, room) + marker;
    if (key === "what-happened") truncated = true;
  }

  return {
    url: assemble(),
    whatHappened: fields["what-happened"] ?? "",
    droppedHardware,
    truncated,
    clipboardText: fullClipboard,
  };
}
