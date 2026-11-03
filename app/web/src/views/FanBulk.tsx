"use client";

import { useRef, useState } from "react";
import { ListChecks, X } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { NumberField } from "@/components/NumberField";
import { Popover } from "@/components/Popover";
import { Select } from "@/components/Select";
import { useToast } from "@/components/Toast";
import { refreshFans } from "@/lib/store";
import { baseFloor } from "@/lib/fan-rules";
import {
  actionWords,
  applyBulk,
  summaryOf,
  type BulkAction,
  type BulkOutcome,
} from "@/lib/fanBulk";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";
import type { FanMode } from "@/proto/FanMode";
import type { FansStatus } from "@/proto/FansStatus";

type Write = (header: string, mode: FanMode, onError: (e: unknown) => void) => Promise<void>;

/**
 * The bar over the Headers list: what is ticked, and the three things to do with it. Its height
 * is reserved at all times, an idle line and the live bar share one cell, so ticking the first
 * header moves nothing.
 */
export function FanBulkBar({
  fans,
  checked,
  focusedId,
  onClear,
  write,
}: {
  fans: FansStatus;
  /** The ticked headers, in list order. */
  checked: FanHeaderStatus[];
  /** The header the editor is on. */
  focusedId: string | null;
  onClear: () => void;
  write: Write;
}) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [percent, setPercent] = useState(50);
  const [pickedFrom, setPickedFrom] = useState<string | null>(null);
  const [result, setResult] = useState<{ words: string; outcomes: BulkOutcome[] } | null>(null);
  const [open, setOpen] = useState(false);
  const detailRef = useRef<HTMLSpanElement>(null);

  const active = checked.length > 0;
  const curves = fans.headers.filter((h) => h.mode.mode === "curve");
  const focusedCurve = curves.find((h) => h.id === focusedId);
  const fromId =
    (pickedFrom && curves.find((h) => h.id === pickedFrom)?.id) ?? focusedCurve?.id ?? curves[0]?.id ?? null;
  const from = curves.find((h) => h.id === fromId);
  const floor = baseFloor("", fans.floor_percent);
  const tach = fans.headers.filter((h) => h.pwm_percent === null);

  const run = async (action: BulkAction) => {
    if (busy || checked.length === 0) return;
    const words = actionWords(action, from?.label);
    setBusy(true);
    setOpen(false);
    try {
      const outcomes = await applyBulk(checked, action, fans, write);
      await refreshFans();
      setResult({ words, outcomes });
      const bad = outcomes.some((o) => o.state === "failed");
      toast(summaryOf(outcomes, words), bad ? "down" : undefined);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fan-bulk" data-active={active ? "true" : "false"} aria-busy={busy}>
      <div className="fan-bulk-idle" aria-hidden={active}>
        <ListChecks size={16} aria-hidden="true" />
        <span>
          Tick headers to set several at once.
          {tach.length > 0 && ` ${tach.map((h) => h.label.replace(/ \(tach only\)$/, "")).join(", ")}: tach only, read-only.`}
        </span>
      </div>

      <div className="fan-bulk-live" inert={!active}>
        <div className="fan-bulk-top">
          <span className="fig fan-bulk-count" role="status" aria-live="polite">
            {checked.length} selected
          </span>
          <span className="fan-bulk-result" role="status" title={result ? summaryOf(result.outcomes, result.words) : undefined}>
            {busy ? "Applying" : result ? summaryOf(result.outcomes, result.words) : ""}
          </span>
          {result && !busy && (
            <span ref={detailRef} className="row-wrap">
              <Button size="sm" variant="ghost" onClick={() => setOpen((o) => !o)}>
                Details
              </Button>
            </span>
          )}
          <Button
            size="sm"
            variant="ghost"
            icon={<X size={14} />}
            onClick={() => {
              setResult(null);
              setOpen(false);
              onClear();
            }}
          >
            Clear
          </Button>
        </div>

        <div className="fan-bulk-actions">
          <Button size="sm" disabled={busy} onClick={() => void run({ kind: "bios" })}>
            BIOS
          </Button>

          <span className="fan-bulk-group">
            <NumberField
              label="Fixed duty for the selection"
              min={floor}
              max={100}
              value={percent}
              unit="%"
              disabled={busy}
              onCommit={setPercent}
            />
            <Button size="sm" disabled={busy} onClick={() => void run({ kind: "fixed", percent })}>
              Set fixed
            </Button>
          </span>

          <span className="fan-bulk-group">
            {from ? (
              <Select
                label="Copy the curve of"
                value={from.id}
                options={curves.map((h) => ({ value: h.id, label: h.label }))}
                disabled={busy}
                onChange={setPickedFrom}
              />
            ) : (
              <span className="mute">No header is on a curve</span>
            )}
            <Button size="sm" disabled={busy || !from} onClick={() => from && void run({ kind: "curve", from: from.id })}>
              Copy curve
            </Button>
          </span>
        </div>
      </div>

      <Popover anchor={detailRef} open={open && result !== null} onClose={() => setOpen(false)} label="What each header did">
        <ul className="fan-bulk-list">
          {result?.outcomes.map((o) => {
            const now = fans.headers.find((h) => h.id === o.id);
            return (
              <li key={o.id} data-state={o.state}>
                <span className="fig fan-bulk-name">{o.label}</span>
                <span className="fan-bulk-state">{o.state}</span>
                <span className="fan-bulk-detail">{o.detail}</span>
                {o.state === "applied" && now && now.pwm_percent !== null && (
                  <span className="fig fan-bulk-duty">
                    runs at {now.pwm_percent}%{now.override_reason ? `, ${now.override_reason}` : ""}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      </Popover>
    </div>
  );
}
