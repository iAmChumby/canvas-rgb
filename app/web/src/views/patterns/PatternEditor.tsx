"use client";

// The pattern editor: one device drawn large as its LEDs, the keys that paint it, the space kept
// under it for a frame strip, and the draft that puts every stroke on the real lights.
//
// The draft is the effect editor's own (`PUT /effects/draft` with a `pattern`): the page opens it
// with its own epoch and a rising sequence, renews it on the heartbeat, and deletes it when the
// editor goes away, which puts the active scene back. The editor never saves by itself: `onSave`
// hands the whole document up, with every field it does not know still in it.

import { useCallback, useEffect, useRef, useState } from "react";
import { FloppyDisk, X } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { BLACK_HEX, parseHex } from "@/lib/color";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { kindWords } from "@/lib/patternFit";
import { DotCanvas } from "@/views/patterns/DotCanvas";
import { PatternTools } from "@/views/patterns/PatternTools";
import { usePatternEditor } from "@/views/patterns/usePatternEditor";
import type { Pattern } from "@/proto/Pattern";
import s from "@/views/patterns/patterns.module.css";

/** `PATTERN_ID_PREFIX` from the spine. */
const PATTERN_ID_PREFIX = "pattern:";
/** `DRAFT_HEARTBEAT` from the spine, for a daemon that has not answered yet. */
const HEARTBEAT_MS = 5_000;
/** Said when another window has taken the lights: this editor's strokes are not on them. */
const SUPERSEDED = "another window has the lights now; this editor's strokes are not on them";

export function PatternEditor({
  pattern,
  note,
  onSave,
  onCancel,
}: {
  /** The loaded pattern: the whole document, frames, motion and unknown fields included. */
  pattern: Pattern;
  /** A sentence to show beside the device, from the page that opened the editor. */
  note?: string | null;
  /** The document to save: the loaded pattern with this frame's colours replaced. */
  onSave?: (pattern: Pattern) => void | Promise<void>;
  onCancel?: () => void;
}) {
  // The draft's epoch, made once when the editor opens: 16 hex characters, as the daemon's own
  // pages make them.
  const [epoch] = useState(newEpoch);
  const seq = useRef(0);
  const alive = useRef(false);
  const heartbeat = useRef(HEARTBEAT_MS);
  const [daemonNote, setDaemonNote] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  /** Puts a stroke on the lights: the page's epoch and the next sequence. */
  const send = useCallback(
    async (document: Pattern) => {
      seq.current += 1;
      const at = seq.current;
      try {
        const answer = await api.putPatternDraft({ pattern: document, epoch, seq: at });
        if (!alive.current) return;
        if (answer.heartbeat_ms > 0) heartbeat.current = answer.heartbeat_ms;
        if (answer.applied === false) setDaemonNote(SUPERSEDED);
        else if (answer.note) setDaemonNote(answer.note);
        if (answer.active === false) setDaemonNote(SUPERSEDED);
        setProblem(null);
      } catch (error) {
        if (!alive.current) return;
        setProblem(messageOf(error));
      }
    },
    [epoch],
  );

  const draft = useCallback((document: Pattern) => void send(document), [send]);
  const editor = usePatternEditor(pattern, { onDraft: draft });
  /** The document as it is now, for the heartbeat's own PUT. */
  const latest = useRef(editor.pattern);

  useEffect(() => {
    latest.current = editor.pattern;
  }, [editor.pattern]);

  // The draft opens with the editor and closes with it. The heartbeat is the spine's own: a PUT
  // of the current document every `heartbeat_ms`, which is what keeps the lease alive while the
  // owner looks at the device.
  useEffect(() => {
    alive.current = true;
    void send(latest.current);
    let timer: ReturnType<typeof setTimeout> | null = null;
    const beat = () => {
      if (!alive.current) return;
      timer = setTimeout(() => {
        void send(latest.current);
        beat();
      }, Math.max(1_000, heartbeat.current));
    };
    beat();
    return () => {
      alive.current = false;
      if (timer !== null) clearTimeout(timer);
      // Closing the editor ends its draft: the custom scene goes back on the lights. The epoch
      // travels with it, so an editor that lost the lights leaves a newer page's draft alone.
      void api.deleteDraft(epoch).catch(() => undefined);
    };
  }, [epoch, send]);

  const name = pattern.id.startsWith(PATTERN_ID_PREFIX) ? pattern.id.slice(PATTERN_ID_PREFIX.length) : pattern.id;
  const leds = pattern.built_on.led_count;
  const lit = editor.colours.filter((colour) => (parseHex(colour) ?? BLACK_HEX) !== BLACK_HEX).length;
  const line = problem ?? daemonNote ?? (note && note.length > 0 ? note : null);

  /** Hands the document up, and clears the unsaved mark once the save lands. */
  const save = async () => {
    if (!onSave) return;
    try {
      await onSave(editor.pattern);
      editor.markSaved();
    } catch (error) {
      setProblem(messageOf(error));
    }
  };

  return (
    <section className={s.editor} aria-label={`${name}, a pattern`}>
      <header className={s.head}>
        <div className={s.who}>
          <h2 className={s.name}>{name}</h2>
          <span className={s.kind}>{kindWords(pattern.built_on)}</span>
        </div>
        <div className={s.tally}>
          {/* As wide as the total's digits, so the count never resizes its own box. */}
          <span className={s.figure} style={{ minWidth: `${String(leds).length}ch` }}>
            {lit}
          </span>
          <span className={s.tallyWords}>
            of {leds} on
          </span>
          <span className={s.dirtyMark} data-on={editor.dirty ? "true" : "false"} aria-hidden="true" />
          <span className="sr-only">{editor.dirty ? "Unsaved changes" : "No unsaved changes"}</span>
        </div>
        <div className={s.headKeys}>
          {onCancel && (
            <Button variant="ghost" icon={<X size={14} weight="bold" aria-hidden="true" />} onClick={onCancel}>
              Cancel
            </Button>
          )}
          {onSave && (
            <Button
              variant="primary"
              icon={<FloppyDisk size={14} weight="bold" aria-hidden="true" />}
              onClick={() => void save()}
            >
              Save
            </Button>
          )}
        </div>
      </header>

      {/* The daemon's own sentence (no device of this kind, another window has the lights). The row
          keeps its height either way, so the device never moves when a sentence arrives. */}
      <p className={s.noteRow} role="status">
        {line !== null && <span className={problem !== null ? s.problem : s.note}>{line}</span>}
      </p>

      <div className={s.stage}>
        <DotCanvas
          builtOn={pattern.built_on}
          colours={editor.colours}
          label={`${name}, ${kindWords(pattern.built_on)}`}
          onBegin={editor.beginStroke}
          onStroke={editor.stroke}
          onEnd={editor.endStroke}
        />
      </div>

      {/* Reserved: the frame strip comes in a later version. An empty recess, never a control. */}
      <div className={s.frameStrip} aria-hidden="true" />

      <PatternTools
        tool={editor.tool}
        colour={editor.colour}
        canUndo={editor.canUndo}
        canRedo={editor.canRedo}
        onTool={editor.setTool}
        onColour={editor.setColour}
        onFillAll={editor.fillAll}
        onUndo={editor.undo}
        onRedo={editor.redo}
      />
    </section>
  );
}

/** 16 random hex characters, for one editor's draft epoch. */
function newEpoch(): string {
  const bytes = new Uint8Array(8);
  if (typeof globalThis.crypto?.getRandomValues === "function") globalThis.crypto.getRandomValues(bytes);
  else for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}
