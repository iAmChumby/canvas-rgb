"use client";

import { useEffect, useRef, useState } from "react";
import { ListChecks, X } from "@phosphor-icons/react";
import { Button, WaitMark } from "@/components/Button";
import { NumberField } from "@/components/NumberField";
import { Popover } from "@/components/Popover";
import { Select } from "@/components/Select";
import { notify } from "@/components/Notify";
import { refreshFans } from "@/lib/store";
import { baseFloor } from "@/lib/fan-rules";
import {
  actionWords,
  applyBulk,
  isTachOnly,
  plainLabel,
  summaryOf,
  type BulkAction,
  type BulkOutcome,
  type Write,
} from "@/lib/fanBulk";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";
import type { FansStatus } from "@/proto/FansStatus";

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
  onBusy,
  write,
}: {
  fans: FansStatus;
  /** The ticked headers, in list order. */
  checked: FanHeaderStatus[];
  /** The header the editor is on. */
  focusedId: string | null;
  onClear: () => void;
  /** Told when a run starts and ends, so the controls that would drop the bar can wait. */
  onBusy: (busy: boolean) => void;
  write: Write;
}) {
  const [busy, setBusy] = useState(false);
  // Which action is out: its key is busy (spinner, same size); the other controls wait shut.
  const [running, setRunning] = useState<BulkAction["kind"] | null>(null);
  const [percent, setPercent] = useState(50);
  const [pickedFrom, setPickedFrom] = useState<string | null>(null);
  const [result, setResult] = useState<{ words: string; outcomes: BulkOutcome[] } | null>(null);
  const [open, setOpen] = useState(false);
  const detailRef = useRef<HTMLSpanElement>(null);
  // The run belongs to the bar. If the bar goes away mid-run, the headers not yet written are not
  // written, and a toast says what was applied before it stopped.
  const alive = useRef(true);
  const stop = useRef<AbortController | null>(null);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      stop.current?.abort();
    };
  }, []);
  useEffect(() => {
    onBusy(busy);
    return () => onBusy(false);
  }, [busy, onBusy]);

  const active = checked.length > 0;
  const curves = fans.headers.filter((h) => h.mode.mode === "curve");
  const focusedCurve = curves.find((h) => h.id === focusedId);
  const fromId =
    (pickedFrom && curves.find((h) => h.id === pickedFrom)?.id) ?? focusedCurve?.id ?? curves[0]?.id ?? null;
  const from = curves.find((h) => h.id === fromId);
  const floor = baseFloor("", fans.floor_percent);
  // A header with no duty is either one the daemon calls tach only, or one whose duty was not
  // read (#279). The bar names each for what it is.
  const noDuty = fans.headers.filter((h) => h.available && h.pwm_percent === null);
  const tach = noDuty.filter(isTachOnly);
  const unread = noDuty.filter((h) => !isTachOnly(h));

  const failed = result?.outcomes.some((o) => o.state === "failed") ?? false;

  const run = async (action: BulkAction) => {
    if (busy || checked.length === 0) return;
    const words = actionWords(action, from?.label);
    const ctl = new AbortController();
    stop.current = ctl;
    setBusy(true);
    setRunning(action.kind);
    setOpen(false);
    try {
      const outcomes = await applyBulk(checked, action, fans, write, ctl.signal);
      await refreshFans();
      if (alive.current) {
        setResult({ words, outcomes });
      } else {
        // The bar is gone, so the report has nowhere to stand: the notification is all that is
        // left of it.
        const bad = outcomes.some((o) => o.state === "failed") || outcomes.length < checked.length;
        notify(summaryOf(outcomes, words, checked.length), bad ? "down" : "ok");
      }
    } finally {
      stop.current = null;
      if (alive.current) {
        setBusy(false);
        setRunning(null);
      }
    }
  };

  return (
    <div className="fan-bulk" data-active={active ? "true" : "false"} aria-busy={busy}>
      <div className="fan-bulk-idle" aria-hidden={active}>
        <ListChecks size={16} aria-hidden="true" />
        <span>
          Tick headers to set several at once.
          {tach.length > 0 && ` ${tach.map((h) => plainLabel(h.label)).join(", ")}: tach only, read-only.`}
          {unread.length > 0 && ` ${unread.map((h) => h.label).join(", ")}: duty not read, cannot be set.`}
        </span>
      </div>

      <div className="fan-bulk-live" inert={!active}>
        <div className="fan-bulk-top">
          <span className="fig fan-bulk-count" role="status" aria-live="polite">
            {checked.length} selected
          </span>
          <span className="fan-bulk-result" role="status" data-tone={failed ? "down" : "ok"}>
            {busy ? <WaitMark busy label="Applying" /> : result ? summaryOf(result.outcomes, result.words) : ""}
          </span>
          {result && !busy && (
            <span ref={detailRef} className="row-wrap fan-bulk-details">
              <Button size="sm" variant="ghost" onClick={() => setOpen((o) => !o)}>
                Details
              </Button>
            </span>
          )}
          <Button
            size="sm"
            variant="ghost"
            className="fan-bulk-clear"
            icon={<X size={14} />}
            disabled={busy}
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
          <Button
            size="sm"
            busy={running === "bios"}
            disabled={busy && running !== "bios"}
            onClick={() => void run({ kind: "bios" })}
          >
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
            <Button
              size="sm"
              busy={running === "fixed"}
              disabled={busy && running !== "fixed"}
              onClick={() => void run({ kind: "fixed", percent })}
            >
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
            <Button
              size="sm"
              busy={running === "curve"}
              disabled={(busy && running !== "curve") || !from}
              onClick={() => from && void run({ kind: "curve", from: from.id })}
            >
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
