"use client";

import { useEffect, useState } from "react";
import { ArrowRight } from "@phosphor-icons/react";
import { Button, Swap } from "@/components/Button";
import { Panel, StateDot } from "@/components/Panel";
import { ErrorPanel, LoadFrame } from "@/components/Loading";
import { UpdatesShape } from "@/components/LoadShapes";
import { canUpdate, openLogsFolder, tauriHost, updateCheckNow, updateInstall, updateSkip } from "@/lib/host";
import { parseInline, parseNotes, type NoteBlock } from "@/lib/releaseNotes";
import { messageOf } from "@/lib/errors";
import { refreshUpdate, useSnap } from "@/lib/store";
import { sendUpdate } from "@/lib/updateActions";
import {
  KEY_LABELS,
  LONGEST_STATE,
  downloadingLabel,
  fmtChecked,
  isInstalling,
  isSkipped,
  keyStep,
  slotFill,
  stateLine,
  stateTone,
  uacLine,
} from "@/lib/updateWords";
import type { UpdateState } from "@/proto/UpdateState";

/** How long the tray has to answer before the page says that it did not. */
export const TRAY_TIMEOUT_MS = 5000;

/** The panel title tone: what a heading of the changelog means, as a token. */function noteTone(title: string): string {
  const t = title.toLowerCase();
  if (t.startsWith("add")) return "var(--ok)";
  if (t.startsWith("fix")) return "var(--warn)";
  if (t.startsWith("remov") || t.startsWith("deprecat") || t.startsWith("secur")) return "var(--down)";
  return "var(--work)";
}

function NotePanel({ block }: { block: NoteBlock }) {
  return (
    <Panel title={block.title}>
      <ul className="notes-list" style={{ "--tone": noteTone(block.title) } as React.CSSProperties}>
        {block.items.map((item, i) => (
          <li key={i}>
            {parseInline(item).map((p, j) =>
              p.kind === "code" ? (
                <code key={j}>{p.text}</code>
              ) : p.kind === "strong" ? (
                <strong key={j}>{p.text}</strong>
              ) : (
                <span key={j}>{p.text}</span>
              ),
            )}
          </li>
        ))}
      </ul>
    </Panel>
  );
}

/** The Update now key, and the 4 px slot under it that fills from real bytes. No spinner. */
function UpdateKey({ u }: { u: UpdateState }) {
  const [err, setErr] = useState<string | null>(null);
  const step = keyStep(u);
  const fill = slotFill(u);
  const busy = isInstalling(u);
  const acts = canUpdate();
  return (
    <>
      <div className="upd-key">
        <Button
          variant="primary"
          aria-busy={busy || undefined}
          disabled={busy || !acts}
          onClick={() => {
            setErr(null);
            void sendUpdate(updateInstall).then(setErr);
          }}
        >
          <Swap labels={KEY_LABELS} at={step} shown={step === 1 ? downloadingLabel(u) : undefined} />
        </Button>
        <div
          className="upd-slot"
          role="progressbar"
          aria-label="Update progress"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={fill === null ? undefined : Math.round(fill * 100)}
        >
          <div
            className="upd-slot-fill"
            data-unknown={fill === null ? "true" : undefined}
            style={{ "--p": fill ?? 1 } as React.CSSProperties}
          />
        </div>
      </div>
      {err && <p className="error-line">{err}</p>}
    </>
  );
}

/** The state line. A hidden copy of the longest line shares its cell, so the line is as tall as it can get. */
function StateLine({ u }: { u: UpdateState }) {
  const line = stateLine(u);
  return (
    <p className="upd-state" role="status" aria-live="polite">
      <span className="upd-state-cur">
        <StateDot tone={stateTone(u)}>{line}</StateDot>
      </span>
      <span className="upd-state-size" aria-hidden="true">
        <StateDot tone="mute">{line.length > LONGEST_STATE.length ? line : LONGEST_STATE}</StateDot>
      </span>
    </p>
  );
}

export function UpdatesView() {
  const u = useSnap((s) => s.update);
  const [err, setErr] = useState<string | null>(null);
  // The tray has not answered for this long: the page says so, instead of waiting for good.
  const [silent, setSilent] = useState(false);
  // How many times the page has asked the tray. Every ask gets its own deadline: a Try again
  // must end in an answer or in the error again, never in a placeholder that waits for good.
  const [asked, setAsked] = useState(0);

  useEffect(() => {
    if (u) return;
    const timer = setTimeout(() => setSilent(true), TRAY_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [u, asked]);

  if (!u) {
    return (
      <div className="stack">
        <div className="page-head">
          <h1>Updates</h1>
        </div>
        {/* The state comes from the tray, not the daemon: there is no error to show while the tray
            is still answering. When it stays quiet the page says so and offers a way to ask again. */}
        <LoadFrame
          state={silent ? "error" : "loading"}
          shape={<UpdatesShape />}
          label="Reading the update state"
          error={
            <ErrorPanel
              title="No answer from the tray"
              summary="The tray app did not answer. Start it, then try again."
              onRetry={() => {
                setSilent(false);
                setAsked((n) => n + 1);
                void refreshUpdate();
              }}
            />
          }
        />
      </div>
    );
  }

  const avail = u.available;
  const notesOf = avail ?? u.updated_to;
  const blocks = notesOf ? parseNotes(notesOf.notes) : [];
  const acts = canUpdate();
  const busy = isInstalling(u);

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Updates</h1>
        <span className="mute fig">
          {u.current_version}
          {avail && (
            <>
              {" "}
              <ArrowRight size={12} className="upd-arrow" aria-hidden="true" /> {avail.version}
            </>
          )}
        </span>
      </div>

      <div className="settings-grid">
        <div className="stack">
          {avail ? (
            <Panel title={`canvas-rgb ${avail.version}`}>
              <UpdateKey u={u} />
              <StateLine u={u} />
              <p className="upd-uac">{uacLine(avail.version)}</p>
              <div className="row-wrap upd-more">
                {!isSkipped(u) && (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy || !acts}
                    onClick={() => {
                      setErr(null);
                      void sendUpdate(updateSkip).then(setErr);
                    }}
                  >
                    Skip this version
                  </Button>
                )}
                {isSkipped(u) && <span className="mute upd-note">{avail.version} is skipped. Update now still works.</span>}
                {u.phase.kind === "install_failed" && (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={tauriHost() === null}
                    onClick={() => {
                      setErr(null);
                      openLogsFolder().catch((e: unknown) => setErr(messageOf(e)));
                    }}
                  >
                    Open logs folder
                  </Button>
                )}
                {(!acts || (u.phase.kind === "install_failed" && tauriHost() === null)) && (
                  <span className="mute upd-note">Tray app only</span>
                )}
              </div>
              {err && <p className="error-line">{err}</p>}
            </Panel>
          ) : (
            <Panel title="Up to date">
              <StateLine u={u} />
              <div className="row-wrap upd-more">
                <Button
                  disabled={u.phase.kind === "checking" || isInstalling(u) || !acts}
                  onClick={() => {
                    setErr(null);
                    void sendUpdate(updateCheckNow).then(setErr);
                  }}
                >
                  <Swap labels={["Check now", "Checking"]} at={u.phase.kind === "checking" ? 1 : 0} />
                </Button>
                <span className="mute upd-note">
                  {acts ? `Last checked ${fmtChecked(u.last_check)}` : "Tray app only"}
                </span>
              </div>
              {err && <p className="error-line">{err}</p>}
            </Panel>
          )}
        </div>

        <div className="stack">
          {notesOf &&
            (blocks.length > 0 ? (
              blocks.map((b, i) => <NotePanel key={`${b.title}-${i}`} block={b} />)
            ) : (
              <Panel title="What's new">
                <p className="mute">No notes were published for {notesOf.version}.</p>
              </Panel>
            ))}
        </div>
      </div>
    </div>
  );
}
