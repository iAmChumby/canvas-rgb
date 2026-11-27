"use client";

import { useCallback, useEffect, useState } from "react";
import { Check, Copy, FolderOpen, DownloadSimple } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { ErrorState } from "@/components/ErrorState";
import { Panel } from "@/components/Panel";
import { InlineSkeleton } from "@/components/Skeleton";
import { diagBundleList, diagBundleSave, revealDiagBundle, tauriHost } from "@/lib/host";
import type { DiagBundle } from "@/proto/DiagBundle";
import s from "./DiagnosticsPanel.module.css";

/**
 * Save diagnostics (#575): the person sees the file list and sizes before saving, then the path,
 * the count of replaced names and a reveal/copy key. One code path serves Help and the
 * daemon-down page; both call the tray, so it works with the daemon down.
 */
export function DiagnosticsPanel({ compact = false }: { compact?: boolean }) {
  const [list, setList] = useState<DiagBundle | null>(null);
  const [saved, setSaved] = useState<DiagBundle | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const available = tauriHost() !== null;

  const load = useCallback(() => {
    diagBundleList()
      .then((bundle) => {
        setList(bundle);
        setError(null);
      })
      .catch((e: unknown) => setError(e));
  }, []);

  useEffect(() => {
    if (available) load();
  }, [available, load]);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      setSaved(await diagBundleSave());
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  };

  const body = (
    <div className={s.body}>
      {!available ? (
        <ErrorState message="Save diagnostics is available in the tray app only." />
      ) : error !== null ? (
        <>
          <ErrorState error={error} onRetry={load} logsEnabled={false} />
        </>
      ) : (
        <>
          {saved === null ? (
            <>
              <ul className={s.files} aria-label="Files in the diagnostics zip">
                {(list?.files ?? []).map((file) => (
                  <li key={file.name}>
                    <span className={s.name}>{file.name}</span>
                    <span className={`fig ${s.size}`}>{formatBytes(file.bytes)}</span>
                  </li>
                ))}
                {list === null ? <li className={s.waiting} aria-busy="true"><InlineSkeleton width="24ch" /></li> : null}
                {serviceLine(list?.service_logs) ? <li className={s.reason}>{serviceLine(list?.service_logs)}</li> : null}
              </ul>
              <Button id="diag-save" icon={<DownloadSimple size={16} />} busy={busy} onClick={() => void save()}>
                {compact ? "Save the zip" : "Save diagnostics"}
              </Button>
            </>
          ) : (
            <>
              <p className={s.saved}>
                <Check size={15} weight="bold" aria-hidden="true" />
                Saved. {saved.replaced} name{saved.replaced === 1 ? "" : "s"} and device ids replaced.
              </p>
              <p className={`fig ${s.path}`} title={saved.path ?? undefined}>
                {saved.path ?? "Saved"}
              </p>
              {serviceLine(saved.service_logs) ? <p className={s.reason}>{serviceLine(saved.service_logs)}</p> : null}
              <div className={s.keys}>
                <Button
                  size="sm"
                  icon={<FolderOpen size={15} />}
                  onClick={() => void revealDiagBundle(saved.path ?? "").catch((e: unknown) => setError(e))}
                >
                  Reveal in Explorer
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  icon={copied ? <Check size={15} /> : <Copy size={15} />}
                  onClick={() => {
                    void navigator.clipboard
                      .writeText(saved.path ?? "")
                      .then(() => setCopied(true))
                      .catch((e: unknown) => setError(e));
                  }}
                >
                  {copied ? "Copied" : "Copy path"}
                </Button>
              </div>
            </>
          )}
        </>
      )}
    </div>
  );

  return compact ? body : <Panel title="Save diagnostics">{body}</Panel>;
}

/** The daemon's refusal already begins with "Service logs": never print the label twice (U12). */
function serviceLine(reason: string | null | undefined): string | null {
  if (!reason) return null;
  return reason.startsWith("Service logs:") ? reason : `Service logs: ${reason}`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
