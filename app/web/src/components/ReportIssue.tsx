"use client";

import { useMemo, useState } from "react";
import { Check, Copy, ArrowSquareOut } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { openExternal } from "@/lib/host";
import { buildReportUrl, REPORT_URL_MAX } from "@/lib/reportUrl";
import type { AboutReport } from "@/proto/AboutReport";
import s from "./ReportIssue.module.css";

/**
 * Report issue (#575, item 6, U10): the preview shows the exact text the address will carry and
 * every filled field, with the count of encoded characters used. The address opens only when the
 * person clicks. Copy report stays beside it for a PC with no browser or network. `errorTitle`
 * must be a scrubbed daemon or tray message, never a local `Error` detail (R6).
 */
export function ReportIssue({
  report,
  errorTitle = null,
  compact = false,
}: {
  report: AboutReport | null;
  errorTitle?: string | null;
  compact?: boolean;
}) {
  const built = useMemo(
    () => buildReportUrl(report ?? emptyReport(), errorTitle),
    [report, errorTitle],
  );
  const [copied, setCopied] = useState(false);
  const [failed, setFailed] = useState(false);

  const copy = () => {
    void navigator.clipboard
      .writeText(built.clipboardText)
      .then(() => {
        setCopied(true);
        setFailed(false);
      })
      .catch(() => {
        setFailed(true);
        setCopied(false);
      });
  };

  const filled = report ?? emptyReport();
  const fields: Array<[string, string]> = [
    ["version", filled.version || "not collected"],
    ["windows", filled.windows || "not collected"],
    ["hardware", built.droppedHardware ? "dropped to fit the address" : filled.hardware || "not collected"],
    ["other-rgb-apps", filled.other_rgb_apps || "not collected"],
    ["what-happened", built.whatHappened || "add your own words"],
  ];

  return (
    <div className={s.body}>
      <p className={s.note}>
        The issue opens in your browser with these fields filled. Nothing is sent until you post the
        issue. {built.url.length} of {REPORT_URL_MAX} characters used
        {built.droppedHardware ? " (hardware left out to fit)" : ""}.
      </p>
      <pre className={s.preview} aria-label="Report issue preview">
        {fields.map(([name, value]) => `${name}: ${value}`).join("\n")}
      </pre>
      {built.droppedHardware ? (
        <p className={s.warn}>Hardware was dropped to fit the address. Add it by hand; Copy report has the full text.</p>
      ) : null}
      {built.truncated ? <p className={s.warn}>What happened was truncated. Copy report has the full text.</p> : null}
      <div className={s.keys}>
        <Button icon={<ArrowSquareOut size={16} />} onClick={() => void openExternal(built.url)} disabled={!report}>
          Report issue
        </Button>
        <Button variant="ghost" icon={copied ? <Check size={16} /> : <Copy size={16} />} onClick={copy}>
          {copied ? "Copied" : "Copy report"}
        </Button>
        {!compact ? <span className={s.learn}>Logs are not carried in an address: drag the diagnostics zip into the issue.</span> : null}
      </div>
      {failed ? <p className={s.warn}>The clipboard refused the copy.</p> : null}
    </div>
  );
}

function emptyReport(): AboutReport {
  return { markdown: "", version: "", windows: "", hardware: "", other_rgb_apps: "" };
}
