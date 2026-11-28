"use client";

import { useState } from "react";
import { Check, EyeSlash } from "@phosphor-icons/react";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { Button } from "@/components/Button";
import { notify } from "@/components/Notify";
import { Figure } from "@/components/Readout";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { doneCount, setupFinished, setupRows, type SetupRow } from "@/lib/setup";
import { refreshStatus, useSnap } from "@/lib/store";
import { closeSetup, putOffSetup, welcomeBack } from "@/views/editor/flow";
import s from "@/views/setup/setup.module.css";

/** The rows as the latest answers have them. */
export function useSetupRows(): SetupRow[] {
  const status = useSnap((st) => st.status);
  const fans = useSnap((st) => st.fans);
  const fansError = useSnap((st) => st.fansError);
  const effects = useSnap((st) => st.effects);
  return setupRows({ status, fans, fansError, effects });
}

/** One lamp. The row's words say its state; the lamp agrees. */
export function Led({ state }: { state: SetupRow["state"] }) {
  return <span className={s.led} data-state={state} aria-hidden="true" />;
}

const STATE_WORDS: Record<SetupRow["state"], string> = {
  done: "Done",
  todo: "Not yet",
  unknown: "Cannot check",
  absent: "Not on this PC",
};

/** "2/4", counting. A dash until the daemon has answered. */
export function SetupCount({ rows }: { rows: SetupRow[] }) {
  const known = useSnap((st) => st.status !== null);
  return (
    <Figure>
      {known ? <AnimatedNumber value={doneCount(rows)} /> : "—"}/{rows.length}
    </Figure>
  );
}

function Row({ row, onTroubleshoot }: { row: SetupRow; onTroubleshoot?: () => void }) {
  return (
    <li className={s.row} data-row={row.id} data-state={row.state}>
      <Led state={row.state} />
      <span className={s.name}>
        {row.name}
        <span className="sr-only">: {STATE_WORDS[row.state]}</span>
      </span>
      {row.id === "lights" && row.state === "todo" && onTroubleshoot && (
        <Button size="sm" variant="ghost" className={s.rowKey} onClick={onTroubleshoot}>
          Fix
        </Button>
      )}
      <span className={s.why}>{row.why}</span>
      <span className={s.value} title={row.value}>
        {row.value}
      </span>
    </li>
  );
}

/**
 * The checklist: a heading, the count, four rows that never move or change height, and the ways
 * out. "Set up later" shuts it for this launch. The other key ends the onboarding for good: "Done"
 * once every row that can be done is done, and "Hide checklist" until then, so it is never out of
 * reach.
 */
export function SetupChecklist({
  onTroubleshoot,
  onLeave,
}: {
  /** Opens the hello's ladder of things to try, for a PC with no lights. */
  onTroubleshoot?: () => void;
  /** Called after either way out. */
  onLeave?: () => void;
}) {
  const status = useSnap((st) => st.status);
  const rows = useSetupRows();
  const [busy, setBusy] = useState(false);
  const finished = setupFinished(rows);
  const title = finished ? "Set up." : welcomeBack(status, rows) ? "Welcome back" : "Setup checklist";

  const complete = async () => {
    setBusy(true);
    try {
      await api.completeOnboarding();
      await refreshStatus();
      closeSetup();
      onLeave?.();
    } catch (e) {
      notify(messageOf(e), "warn");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={s.body}>
      <div className={s.head}>
        <h2 className={s.title}>{title}</h2>
        <span className={s.count} data-testid="setup-count">
          <SetupCount rows={rows} />
        </span>
      </div>
      <ol className={s.rows}>
        {rows.map((row) => (
          <Row key={row.id} row={row} onTroubleshoot={onTroubleshoot} />
        ))}
      </ol>
      {/* One row of fixed height, one key on the right in every state: "Done" arriving moves nothing. */}
      <div className={s.keys}>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            putOffSetup();
            onLeave?.();
          }}
        >
          Set up later
        </Button>
        {finished && <Button
          size="sm"
          variant={finished ? "primary" : "default"}
          busy={busy}
          icon={finished ? <Check size={14} weight="bold" /> : <EyeSlash size={14} />}
          onClick={() => void complete()}
        >
          {finished ? "Done" : "Hide checklist"}
        </Button>}
      </div>
    </div>
  );
}
