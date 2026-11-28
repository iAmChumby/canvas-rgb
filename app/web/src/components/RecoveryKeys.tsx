"use client";

import { useState } from "react";
import { Copy, FolderOpen, DownloadSimple, ArrowSquareOut } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { ErrorState, KeyRow } from "@/components/ErrorState";
import { DiagnosticsPanel } from "@/components/DiagnosticsPanel";
import { InlineSkeleton } from "@/components/Skeleton";
import { ReportIssue } from "@/components/ReportIssue";
import { isDemo, openLogsFolder, openServiceLogsFolder, reportPrefill } from "@/lib/host";
import { caught, type Failure } from "@/lib/errors";
import type { AboutReport } from "@/proto/AboutReport";

/**
 * The key row on DaemonDown and EngineDown (item 3, R4): what a person can do from here. The
 * panels stay collapsed until the key is pressed, so the page keeps its height and no error
 * lives only in a toast. The daemon is not needed for the tray's two panels.
 */
export function RecoveryKeys({
  title,
  detail,
  version = null,
  withServiceLogs = false,
}: {
  title: string;
  detail?: string | null;
  version?: string | null;
  withServiceLogs?: boolean;
}) {
  const [panel, setPanel] = useState<"none" | "diag" | "report">("none");
  const [report, setReport] = useState<AboutReport | null>(null);
  const [refused, setRefused] = useState<Failure | null>(null);
  const [copied, setCopied] = useState(false);
  const tray = !isDemo();

  const showReport = () => {
    setPanel("report");
    if (!report) {
      void reportPrefill()
        .then((prefill) =>
          setReport({
            markdown: "",
            version: prefill.version,
            windows: prefill.windows,
            hardware: "not collected: daemon not running",
            other_rgb_apps: "not collected: daemon not running",
          }),
        )
        .catch((e: unknown) => setRefused(caught(e)));
    }
  };

  const copyDetails = () => {
    void navigator.clipboard
      .writeText([title, detail, version].filter(Boolean).join("\n"))
      .then(() => setCopied(true))
      .catch(() => setRefused("The clipboard refused the copy."));
  };

  return (
    <div className="recovery-keys">
      <div className="recovery-keys-row">
        <KeyRow
          icon={FolderOpen}
          label="Open logs folder"
          disabled={!tray}
          onClick={() => void openLogsFolder().catch((e: unknown) => setRefused(caught(e)))}
        />
        {withServiceLogs ? (
          <KeyRow
            icon={FolderOpen}
            label="Open service logs"
            disabled={!tray}
            onClick={() => void openServiceLogsFolder().catch((e: unknown) => setRefused(caught(e)))}
          />
        ) : null}
        <KeyRow icon={DownloadSimple} label="Save diagnostics" disabled={!tray} onClick={() => setPanel(panel === "diag" ? "none" : "diag")} />
        <KeyRow icon={ArrowSquareOut} label="Report issue" onClick={showReport} />
        <Button size="sm" variant="ghost" icon={copied ? <Copy size={15} /> : <Copy size={15} />} onClick={copyDetails}>
          {copied ? "Copied details" : "Copy details"}
        </Button>
      </div>
      {refused ? <ErrorState compact error={refused} /> : null}
      {panel === "diag" ? <DiagnosticsPanel compact /> : null}
      {panel === "report" ? (
        <div className="recovery-panel">
          {report === null ? (
            <p aria-busy="true"><InlineSkeleton width="24ch" /></p>
          ) : (
            <ReportIssue report={report} errorTitle={detail ?? null} compact />
          )}
        </div>
      ) : null}
    </div>
  );
}
