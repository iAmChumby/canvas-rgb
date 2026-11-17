"use client";

// The pattern editor: one device drawn large as its LEDs, the keys that paint it, the space kept
// under it for a frame strip, and the draft that puts every stroke on the real lights.
//
// The draft is the effect editor's own (`PUT /effects/draft` with a `pattern`): the page opens it
// with its own epoch and a rising sequence, renews it on the heartbeat, and deletes it when the
// editor goes away, which puts the active scene back. The editor never saves by itself: `onSave`
// hands the whole document up, with every field it does not know still in it.
//
// On a page of its own the editor guards that page (#474): strokes that are not saved are never
// dropped without asking, on the Cancel key, on a rail link, on the browser's Back and on a
// closed window. The prompt is the stack builder's own, with the same words.

import { useCallback, useEffect, useRef, useState } from "react";
import { FloppyDisk, X } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { LeavePrompt } from "@/components/LeavePrompt";
import { ShapePicker, type Shape } from "@/components/ShapePicker";
import { useFocusTrap } from "@/components/useFocusTrap";
import { BLACK_HEX, parseHex } from "@/lib/color";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { cancelNavigation, continueNavigation, hasPendingNavigation } from "@/lib/navigationGuard";
import { kindWords } from "@/lib/patternFit";
import { spinOf } from "@/lib/spin";
import { refreshLights, useSnap } from "@/lib/store";
import { useLeaveGuard, type LeaveReason } from "@/lib/useLeaveGuard";
import { DotCanvas } from "@/views/patterns/DotCanvas";
import { GradientStops } from "@/views/patterns/GradientStops";
import { MotionControl } from "@/views/patterns/MotionControl";
import { PatternTools } from "@/views/patterns/PatternTools";
import { gridOfZone, referenceLight, shapesOf, zoneOf } from "@/views/patterns/shapeLight";
import { usePatternEditor } from "@/views/patterns/usePatternEditor";
import type { Pattern } from "@/proto/Pattern";
import s from "@/views/patterns/patterns.module.css";

/** `PATTERN_ID_PREFIX` from the spine. */
const PATTERN_ID_PREFIX = "pattern:";
/** `DRAFT_HEARTBEAT` from the spine, for a daemon that has not answered yet. */
const HEARTBEAT_MS = 5_000;
/** Said when another window has taken the lights: this editor's strokes are not on them. */
const SUPERSEDED = "another window has the lights now; this editor's strokes are not on them";
/** What Tab reaches inside the prompt, in document order. */
const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled])';

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
  /** A save that did not land: the document on the lights is not the saved one. */
  const [saveProblem, setSaveProblem] = useState<string | null>(null);
  /** How many answers in a row said this editor's stroke is not on the lights. */
  const notApplied = useRef(0);
  /** A save is in flight. */
  const [saving, setSaving] = useState(false);
  /** Why the prompt is open: the Cancel key, an in-app page change, or the browser's Back. */
  const [leaving, setLeaving] = useState<LeaveReason | false>(false);
  /** A Save that will leave the editor: the guard's entry stays while it lands. */
  const pendingLeave = useRef(false);
  const promptRef = useRef<HTMLDivElement | null>(null);
  /** The control the prompt opened from: focus goes back there when it closes. */
  const opener = useRef<HTMLElement | null>(null);

  /** Puts a stroke on the lights: the page's epoch and the next sequence. */
  const send = useCallback(
    async (document: Pattern) => {
      seq.current += 1;
      const at = seq.current;
      /**
       * True when the answer says the lights are another page's. A stroke the daemon answered
       * `applied: false` because a newer stroke of this same page went first is not another
       * window: the answer names the epoch that owns the draft, and only a different one is a
       * supersede. When it names none, two answers in a row are the only sign there is.
       */
      const anotherWindow = (answer: { applied?: boolean; active?: boolean; epoch?: string | null }): boolean => {
        if (answer.applied !== false && answer.active !== false) {
          notApplied.current = 0;
          return false;
        }
        notApplied.current += 1;
        if (answer.epoch !== undefined && answer.epoch !== null) return answer.epoch !== epoch;
        return notApplied.current >= 2;
      };
      try {
        const answer = await api.putPatternDraft({ pattern: document, epoch, seq: at });
        if (!alive.current) return;
        if (answer.heartbeat_ms > 0) heartbeat.current = answer.heartbeat_ms;
        if (anotherWindow(answer)) setDaemonNote(SUPERSEDED);
        else if (answer.note) setDaemonNote(answer.note);
        // A stroke that landed makes an earlier "another window has the lights" note stale.
        else if (answer.applied === true) setDaemonNote(null);
        setProblem(null);
      } catch (error) {
        if (!alive.current) return;
        setProblem(messageOf(error));
      }
    },
    [epoch],
  );

  /**
   * The newest document, for the heartbeat's own PUT: moved with every draft that goes out, so a
   * beat that lands between a stroke and its render sends that stroke and never the colours the
   * render began in.
   */
  const latest = useRef<Pattern>(pattern);
  const draft = useCallback(
    (document: Pattern) => {
      latest.current = document;
      void send(document);
    },
    [send],
  );
  const editor = usePatternEditor(pattern, { onDraft: draft });

  // The shape picker writes the arrangement of the light this pattern was built on (#468).
  const lights = useSnap((state) => state.lights);
  const devices = useSnap((state) => state.devices);
  const light = referenceLight(editor.builtOn, lights, devices);
  const zone = zoneOf(light, devices);
  const [shapeProblem, setShapeProblem] = useState<string | null>(null);
  const chooseShape = async (shape: Shape) => {
    setShapeProblem(null);
    if (light && light.light.arrangement !== shape) {
      try {
        await api.lights.patch(light.light.id, { arrangement: shape });
        await refreshLights();
      } catch (error) {
        // The light did not change, so neither does the pattern's shape: the header says why.
        setShapeProblem(messageOf(error));
        return;
      }
    }
    editor.setShape(shape, gridOfZone(zone));
  };

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
  const leds = editor.builtOn.led_count;
  const lit = editor.colours.filter((colour) => (parseHex(colour) ?? BLACK_HEX) !== BLACK_HEX).length;
  const line = saveProblem ?? shapeProblem ?? problem ?? daemonNote ?? (note && note.length > 0 ? note : null);
  const failed = saveProblem !== null || shapeProblem !== null || problem !== null;

  // The guard: the shell asks it before it changes page, the browser's Back lands on an entry
  // of the editor's own, and the browser is warned when the window closes. The editor asks
  // wherever it stands: on its own page and in the effects view's panel alike.
  const { allowBack } = useLeaveGuard({
    dirty: editor.dirty,
    sentinel: "canvasRgbPatternGuard",
    onAsk: (reason) => setLeaving(reason),
    isLeaving: () => pendingLeave.current,
  });

  /**
   * Hands the document up and clears the unsaved mark only once the save lands: a save that fails
   * says why in the row the editor's other notes use, and leaves the strokes and the mark as they
   * are, so nothing painted is lost and Save can be pressed again. `then` runs after a save that
   * landed: the prompt's Save carries the leave out.
   */
  const save = async (then?: () => void) => {
    if (!onSave) return;
    setSaving(true);
    let landed = false;
    try {
      await onSave(editor.pattern);
      setSaveProblem(null);
      editor.markSaved();
      landed = true;
    } catch (error) {
      // The save failed, so the editor is not leaving after all.
      pendingLeave.current = false;
      setSaveProblem(messageOf(error));
    }
    setSaving(false);
    if (landed) then?.();
  };

  /**
   * Carries out the leave the prompt was asked about, as it was asked: a Back the guard caught is
   * not the same leave as the Cancel key pressed afterwards.
   */
  const goAfterLeave = (reason: LeaveReason) => {
    pendingLeave.current = true;
    if (reason === "back") {
      // Back goes past the guard's entry and past the page itself, one page in all.
      allowBack();
      window.history.go(-2);
      return;
    }
    // A rail link the shell held is the page change that goes now.
    if (hasPendingNavigation()) {
      continueNavigation();
      return;
    }
    onCancel?.();
  };

  /** Keep editing: the prompt closes, the held page change is dropped, and the strokes stay. */
  const keepEditing = useCallback(() => {
    setLeaving(false);
    cancelNavigation();
  }, []);

  /** Discard: the strokes go, and the leave happens. */
  const discardLeave = () => {
    const reason = leaving || "close";
    setLeaving(false);
    goAfterLeave(reason);
  };

  /** Save: the pattern is saved, and the leave happens once the save has landed. */
  const saveLeave = () => {
    const reason = leaving || "close";
    setLeaving(false);
    pendingLeave.current = true;
    void save(() => goAfterLeave(reason));
  };

  /** The Cancel key: with strokes to lose, it asks first. */
  const cancel = () => {
    if (editor.dirty) {
      setLeaving("close");
      return;
    }
    onCancel?.();
  };

  // The prompt takes focus when it opens, keeps Tab inside it, and gives the focus back when it
  // closes.
  useEffect(() => {
    if (leaving) {
      const active = document.activeElement;
      opener.current = active instanceof HTMLElement && active !== document.body ? active : null;
      promptRef.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
      return;
    }
    const back = opener.current;
    opener.current = null;
    if (back?.isConnected) back.focus();
  }, [leaving]);

  useFocusTrap({ panelRef: promptRef, onEscape: keepEditing, capture: false, enabled: leaving !== false });

  return (
    <>
    <section className={s.editor} aria-label={`${name}, a pattern`}>
      <header className={s.head}>
        <div className={s.who}>
          <h2 className={s.name}>{name}</h2>
          <span className={s.kind}>{kindWords(editor.builtOn)}</span>
        </div>
        <ShapePicker
          label={`What ${light ? light.label : name} is`}
          value={editor.builtOn.arrangement}
          shapes={shapesOf(editor.builtOn, zone)}
          onChange={(shape) => void chooseShape(shape)}
        />
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
            <Button variant="ghost" icon={<X size={14} weight="bold" aria-hidden="true" />} onClick={cancel}>
              Cancel
            </Button>
          )}
          {onSave && (
            <Button
              variant="primary"
              busy={saving}
              icon={<FloppyDisk size={14} weight="bold" aria-hidden="true" />}
              onClick={() => void save()}
            >
              Save
            </Button>
          )}
        </div>
      </header>

      {/* The daemon's own sentence (no device of this kind, another window has the lights), or a
          save that did not land. The row keeps its height either way, so the device never moves
          when a sentence arrives. */}
      <p className={s.noteRow} role="status">
        {line !== null && <span className={failed ? s.problem : s.note}>{line}</span>}
      </p>

      <div className={s.stage}>
        <DotCanvas
          builtOn={editor.builtOn}
          colours={editor.colours}
          spin={spinOf(editor.motion)}
          label={`${name}, ${kindWords(editor.builtOn)}`}
          onBegin={editor.beginStroke}
          onStroke={editor.stroke}
          onEnd={editor.endStroke}
          overlay={
            editor.tool === "gradient" && editor.gradient ? (
              <GradientStops
                builtOn={editor.builtOn}
                state={editor.gradient}
                onSelect={editor.selectStop}
                onMove={editor.moveStop}
                onBegin={editor.beginStopDrag}
                onEnd={editor.endStopDrag}
              />
            ) : null
          }
        />
      </div>

      <MotionControl motion={editor.motion} onMotion={editor.setMotion} />

      {/* Reserved: the frame strip comes in a later version. An empty recess, never a control. */}
      <div className={s.frameStrip} aria-hidden="true" />

      <PatternTools
        tool={editor.tool}
        colour={editor.colour}
        canUndo={editor.canUndo}
        canRedo={editor.canRedo}
        gradient={editor.gradient}
        onTool={editor.setTool}
        onColour={editor.setColour}
        onFillAll={editor.fillAll}
        onUndo={editor.undo}
        onRedo={editor.redo}
        onWrap={editor.setWrap}
        onAddStop={editor.addStop}
        onRemoveStop={() => editor.gradient && editor.removeStop(editor.gradient.selected)}
      />
    </section>

    {leaving && (
      <LeavePrompt
        promptRef={promptRef}
        saveState={saving ? "saving" : "idle"}
        subject="pattern"
        onKeepEditing={keepEditing}
        onDiscard={discardLeave}
        onSave={saveLeave}
      />
    )}
    </>
  );
}

/** 16 random hex characters, for one editor's draft epoch. */
function newEpoch(): string {
  const bytes = new Uint8Array(8);
  if (typeof globalThis.crypto?.getRandomValues === "function") globalThis.crypto.getRandomValues(bytes);
  else for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}
