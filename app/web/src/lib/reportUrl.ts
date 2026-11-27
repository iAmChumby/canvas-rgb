// The one pre-filled Report issue URL (#575, R7). One string drives the preview, Copy report and
// the link; the link opens only on the person's click, after the preview shows the exact text.
// The cap is on the WHOLE encoded URL: GitHub loads 8100 chars and returns 414 at 8200.

import type { AboutReport } from "@/proto/AboutReport";

/** Total encoded URL budget, measured against GitHub's 414 boundary. */
export const REPORT_URL_MAX = 6000;

const ISSUE_NEW = "https://github.com/iAmChumby/canvas-rgb/issues/new";
const TEMPLATE = "bug_report.yml";

export interface ReportPrefill {
  /** The exact address the button opens and the preview describes. */
  url: string;
  /** The text of the `what-happened` field, also shown in the preview. */
  whatHappened: string;
  /** True when the hardware field was dropped to fit the budget. */
  droppedHardware: boolean;
  /** True when `what-happened` was cut to fit the budget. */
  truncated: boolean;
  /** The full text to copy when the URL could not carry all of it. */
  clipboardText: string;
}

/**
 * Builds the pre-filled address. `errorTitle` must be a daemon or tray message that was scrubbed
 * before it reached the editor; a local `Error` detail is never passed here (R6).
 */
export function buildReportUrl(report: AboutReport, errorTitle?: string | null): ReportPrefill {
  const what = errorTitle ? `Error shown: ${errorTitle}\n\n` : "";
  const fields: Record<string, string> = {
    template: TEMPLATE,
    version: report.version ?? "",
    windows: report.windows ?? "",
    hardware: report.hardware ?? "",
    "other-rgb-apps": report.other_rgb_apps ?? "",
    "what-happened": what,
  };

  const assemble = (): string => {
    const query = Object.entries(fields)
      .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(value)}`)
      .join("&");
    return `${ISSUE_NEW}?${query}`;
  };

  let url = assemble();
  let droppedHardware = false;
  let truncated = false;
  if (url.length > REPORT_URL_MAX) {
    // Drop the largest optional field first: hardware is model-level detail the person can add
    // by hand, while the error and the version are the reason for the report.
    delete fields.hardware;
    droppedHardware = true;
    url = assemble();
  }
  if (url.length > REPORT_URL_MAX) {
    // Then cut `what-happened`, keeping a marker so the preview does not read as complete.
    const kept = fields["what-happened"] ?? "";
    const room = REPORT_URL_MAX - url.length + encodeURIComponent(kept).length - encodeURIComponent(" (truncated)").length - 16;
    const cut = room > 0 ? kept.slice(0, room) : "";
    fields["what-happened"] = `${cut} (truncated)`;
    truncated = true;
    url = assemble();
  }

  return {
    url,
    whatHappened: fields["what-happened"] ?? "",
    droppedHardware,
    truncated,
    clipboardText: `${what}\n\nVersion: ${fields.version}\nWindows: ${fields.windows}\nHardware: ${report.hardware ?? ""}`,
  };
}
