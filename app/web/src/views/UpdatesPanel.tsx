"use client";

import { useEffect, useState } from "react";
import { Button, ButtonLink } from "@/components/Button";
import { Field, StateDot } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { InlineSkeleton } from "@/components/Skeleton";
import { Toggle } from "@/components/Toggle";
import { messageOf } from "@/lib/errors";
import {
  canUpdate,
  getTrayPrefs,
  setEditor,
  setUpdateChannel,
  updateCheckNow,
  updateSetAuto,
  updateUnskip,
} from "@/lib/host";
import { useSnap } from "@/lib/store";
import { sendUpdate } from "@/lib/updateActions";
import { fmtChecked, isExperimentalVersion, isInstalling, isSkipped, settingsStatus } from "@/lib/updateWords";
import type { EditorChoice } from "@/proto/EditorChoice";
import type { TrayPrefs } from "@/proto/TrayPrefs";

/** The editors the tray can open (#202). */
const EDITORS: { value: EditorChoice; label: string }[] = [
  { value: "react", label: "React" },
  { value: "gpui", label: "GPUI" },
];

/**
 * Settings > Updates: the automatic check, when it last ran, Check now, and the status. It is the
 * last panel of the right column, so appearing here moves no other row. Outside the tray there is
 * nothing to ask, so the controls are off and say so, like the logs button.
 */
export function UpdatesPanel() {
  const u = useSnap((s) => s.update);
  const [err, setErr] = useState<string | null>(null);
  // The tray's own preferences: which feed it reads (#203).
  const [prefs, setPrefs] = useState<TrayPrefs | null>(null);
  const acts = canUpdate();

  useEffect(() => {
    // A tray without the command (an older tray, a newer editor) leaves the switch off and
    // disabled: there is nothing to ask.
    void getTrayPrefs().then(setPrefs).catch(() => undefined);
  }, []);

  const run = (call: () => Promise<import("@/proto/UpdateState").UpdateState | null>) => {
    setErr(null);
    void sendUpdate(call).then(setErr);
  };

  // The channel decides which feed the tray reads; the tray checks again by itself.
  const setChannel = (experimental: boolean) => {
    setErr(null);
    void setUpdateChannel(experimental ? "experimental" : "stable")
      .then(setPrefs)
      .catch((e: unknown) => setErr(messageOf(e)));
  };

  // The editor the tray opens (#202). Only an experimental build has the choice: the running
  // version is the one that says so.
  const experimental = u ? isExperimentalVersion(u.current_version) : false;
  const chooseEditor = (editor: EditorChoice) => {
    setErr(null);
    void setEditor(editor)
      .then(setPrefs)
      .catch((e: unknown) => setErr(messageOf(e)));
  };

  const status = u ? settingsStatus(u) : null;
  const checking = u?.phase.kind === "checking";

  return (
    <>
      <Field label="Check automatically" hint="At start, then once a day">
        <Toggle
          label="Check for updates automatically"
          checked={u?.auto_check ?? true}
          disabled={!acts || !u}
          onChange={(v) => run(() => updateSetAuto(v))}
        />
      </Field>
      <Field label="Last checked">
        <span className="fig" aria-busy={(acts && !u) || undefined}>{acts && !u ? <InlineSkeleton width="14ch" /> : acts && u ? fmtChecked(u.last_check) : "—"}</span>
      </Field>
      <Field label="Check now">
        <Button
          busy={checking}
          disabled={!acts || !u || isInstalling(u)}
          onClick={() => run(updateCheckNow)}
        >
          Check now
        </Button>
      </Field>
      <Field label="Experimental builds" hint="Unstable test builds, offered early">
        <Toggle
          label="Experimental builds"
          checked={prefs?.channel === "experimental"}
          disabled={!prefs}
          onChange={setChannel}
        />
      </Field>
      {experimental && (
        <Field label="Editor" hint="GPUI is experimental">
          <Segmented<EditorChoice>
            label="Editor"
            options={EDITORS}
            value={prefs?.editor ?? "react"}
            disabled={!prefs}
            onChange={chooseEditor}
          />
        </Field>
      )}
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
          <span className="mute upd-note" aria-busy={(acts && !u) || undefined}>{acts && !u ? <InlineSkeleton width="18ch" /> : "—"}</span>
        )}
      </div>
      {!acts && <p className="mute upd-note">Tray app only</p>}
      {err && <p className="error-line">{err}</p>}
    </>
  );
}
