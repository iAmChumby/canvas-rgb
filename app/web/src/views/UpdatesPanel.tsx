"use client";

import { useState } from "react";
import { Button, ButtonLink, Swap } from "@/components/Button";
import { Field, Panel, StateDot } from "@/components/Panel";
import { Toggle } from "@/components/Toggle";
import { canUpdate, updateCheckNow, updateSetAuto, updateUnskip } from "@/lib/host";
import { useSnap } from "@/lib/store";
import { sendUpdate } from "@/lib/updateActions";
import { fmtChecked, isInstalling, isSkipped, settingsStatus } from "@/lib/updateWords";

/**
 * Settings > Updates: the automatic check, when it last ran, Check now, and the status. It is the
 * last panel of the right column, so appearing here moves no other row. Outside the tray there is
 * nothing to ask, so the controls are off and say so, like the logs button.
 */
export function UpdatesPanel() {
  const u = useSnap((s) => s.update);
  const [err, setErr] = useState<string | null>(null);
  const acts = canUpdate();

  const run = (call: () => Promise<import("@/proto/UpdateState").UpdateState | null>) => {
    setErr(null);
    void sendUpdate(call).then(setErr);
  };

  const status = u ? settingsStatus(u) : null;
  const checking = u?.phase.kind === "checking";

  return (
    <Panel title="Updates">
      <Field label="Check automatically" hint="At start, then once a day">
        <Toggle
          label="Check for updates automatically"
          checked={u?.auto_check ?? true}
          disabled={!acts || !u}
          onChange={(v) => run(() => updateSetAuto(v))}
        />
      </Field>
      <Field label="Last checked">
        <span className="fig">{acts && u ? fmtChecked(u.last_check) : "—"}</span>
      </Field>
      <Field label="Check now">
        <Button
          disabled={!acts || !u || checking || isInstalling(u)}
          onClick={() => run(updateCheckNow)}
        >
          <Swap labels={["Check now", "Checking"]} at={checking ? 1 : 0} />
        </Button>
      </Field>
      <div className="upd-status">
        {status && u ? (
          <>
            <StateDot tone={status.tone}>{status.text}</StateDot>
            <span className="upd-status-acts">
              {isSkipped(u) && u.phase.kind !== "checking" && (
                <Button size="sm" variant="ghost" disabled={!acts} onClick={() => run(updateUnskip)}>
                  Undo
                </Button>
              )}
              {u.available && !isSkipped(u) && (
                <ButtonLink href="/updates/" size="sm" variant="ghost">
                  What&rsquo;s new
                </ButtonLink>
              )}
            </span>
          </>
        ) : (
          <span className="mute upd-note">—</span>
        )}
      </div>
      {!acts && <p className="mute upd-note">Tray app only</p>}
      {err && <p className="error-line">{err}</p>}
    </Panel>
  );
}
