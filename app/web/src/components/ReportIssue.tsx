"use client";

import { useMemo, useState } from "react";
import { Check, Copy, ArrowSquareOut } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { openExternal } from "@/lib/host";
import { buildReportUrl, REPORT_URL_MAX } from "@/lib/reportUrl";
import type { AboutReport } from "@/proto/AboutReport";
import s from "./ReportIssue.module.css";

/**
 * Report issue (#575, item 6): the preview shows the exact text the address will carry, and the
 * address is opened only on the person's click. Copy report stays beside it for a PC with no
 * browser or network. `errorTitle` must be a scrubbed daemon or tray message, never a local
 * `Error` detail.
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

  return (
    <div className={s.body}>
      <p className={s.note}>
        The issue opens in your browser with these fields filled. Nothing is sent until you post
        the issue. The full report is {built.url.length}/{REPORT_URL_MAX} encoded characters.
      </p>
      <pre className={s.preview} aria-label="Report issue preview">
        {built.whatHappened || "(what happened: add your own words)"}
      </pre>
      {built.droppedHardware ? <p className={s.warn}>Hardware was dropped to fit the address. Add it by hand; the full text is on the clipboard.</p> : null}
      {built.truncated ? <p className={s.warn}>What happened was truncated. Copy report has the full text.</p> : null}
      <div className={s.keys}>
        <Button
          icon={<ArrowSquareOut size={16} />}
          onClick={() => void openExternal(built.url)}
          disabled={!report}
        >
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
