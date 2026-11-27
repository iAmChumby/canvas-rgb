"use client";

// The pattern editor: one device drawn large as its LEDs, the keys that paint it, the frame strip
// under it (#468), and the draft that puts every stroke on the real lights.
//
// The draft is the effect editor's own (`PUT /effects/draft` with a `pattern`): the page opens it
// with its own epoch and a rising sequence, renews it on the heartbeat, and deletes it when the
// editor goes away, which puts the active scene back. The editor never saves by itself: `onSave`
// hands the whole document up, with every field it does not know still in it.
//
// What the lights get is `draftDocument`'s (#481): the selected frame alone while the owner paints,
// the whole timeline while Play is on. The daemon starts a new document's clock when it lands, so
// the preview takes its own clock from the moment the PUT that started the lights was answered.
//
// The editor guards its page (#474), on a page of its own and in the effects view's panel alike:
// strokes that are not saved are never dropped without asking, on the Cancel key, on a rail link,
// on the panel's Back key on a phone, on the browser's Back and on a closed window. The prompt is
// the stack builder's own, with the same words.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowsLeftRight, Crosshair, FloppyDisk, X } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { LedOrderControl } from "@/components/LedOrderControl";
import { directionPatch, effectiveFirstLed, identifyLed } from "@/lib/ledOrder";
import { LeavePrompt } from "@/components/LeavePrompt";
import { Select } from "@/components/Select";
import { ShapePicker, type Shape } from "@/components/ShapePicker";
import { useFocusTrap } from "@/components/useFocusTrap";
import { BLACK_HEX, parseHex } from "@/lib/color";
import { api } from "@/lib/endpoints";
import { errorText } from "@/lib/errors";
import { cancelNavigation, continueNavigation, hasPendingNavigation } from "@/lib/navigationGuard";
import { motionOffNow } from "@/lib/motion";
import { kindWords } from "@/lib/patternFit";
import { draftDocument, timingKey } from "@/lib/patternDraft";
import { isStill, timelineOf } from "@/lib/patternPlay";
import { spinOf } from "@/lib/spin";
import { refreshLights, useSnap } from "@/lib/store";
import { cameFromApp, useLeaveGuard, type LeaveReason } from "@/lib/useLeaveGuard";
import { DotCanvas } from "@/views/patterns/DotCanvas";
import { FrameStrip } from "@/views/patterns/FrameStrip";
import { GradientStops } from "@/views/patterns/GradientStops";
import { MotionControl } from "@/views/patterns/MotionControl";
import { PatternTools } from "@/views/patterns/PatternTools";
import { gridOfZone, choosableLights, shapesOf, zoneOf } from "@/views/patterns/shapeLight";
import { usePatternEditor, type ShapeStep } from "@/views/patterns/usePatternEditor";
import type { Arrangement } from "@/proto/Arrangement";
import type { LightView } from "@/proto/LightView";
import type { Pattern } from "@/proto/Pattern";
import s from "@/views/patterns/patterns.module.css";

/** The `history.state` key of the editor's guard entry; `PATTERN_ENTRY_SPENT` marks one the editor left. */
export const PATTERN_GUARD = "canvasRgbPatternGuard";
export const PATTERN_ENTRY_SPENT = "spent";
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
  ownsEntry = false,
}: {
  /** The loaded pattern: the whole document, frames, motion and unknown fields included. */
  pattern: Pattern;
  /** A sentence to show beside the device, from the page that opened the editor. */
  note?: string | null;
  /** The document to save: the loaded pattern with this frame's colours replaced. */
  onSave?: (pattern: Pattern) => void | Promise<void>;
  onCancel?: () => void;
  /**
   * True when the editor is a page of its own, so its history entry is the editor and nothing
   * else: a rail link replaces that entry, and one Back goes to the page before it. False for the
   * panel in the effects view, whose entry is the effects page itself and stays behind the link.
   */
  ownsEntry?: boolean;
}) {
  // The draft's epoch, made once when the editor opens: 16 hex characters, as the daemon's own
  // pages make them.
  const [epoch] = useState(newEpoch);
  const seq = useRef(0);
  const alive = useRef(false);
  const heartbeat = useRef(HEARTBEAT_MS);
  const [daemonNote, setDaemonNote] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  /** A shape change or its undo that did not reach the light: the pattern keeps the shape it shows. */
  const [shapeProblem, setShapeProblem] = useState<string | null>(null);
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
  /** True when this editor was opened from another page of the app: a Back past the guard's entry
   *  and the editor's own lands there. A direct load has no page behind it. */
  const openedInApp = useRef(cameFromApp());
  const promptRef = useRef<HTMLDivElement | null>(null);
  /** The control the prompt opened from: focus goes back there when it closes. */
  const opener = useRef<HTMLElement | null>(null);

  /**
   * When the lights' clock started, on the page's own clock (`performance.now()`), or null while the
   * PUT that started it has not been answered (#481). The daemon installs a new document, and so
   * starts its clock, when the PUT lands; the middle of the round trip is the best the page can know
   * of that moment without a field in the answer.
   */
  const [lightsClock, setLightsClock] = useState<number | null>(null);
  /** The newest sequence whose answer set the clock, and the one before which no answer may set it. */
  const clockSeq = useRef(0);
  const clockFloor = useRef(0);
  /**
   * The timing of the document the daemon is playing, as the answers have told the page, and the clock
   * it runs on. A PUT of that same document is skipped by the daemon, and a PUT with the same timing
   * keeps the clock (#481, `PatternTiming`), so neither moves the preview's clock.
   */
  const running = useRef<{ timing: string; clock: number } | null>(null);

  /**
   * Puts a stroke on the lights: the page's epoch and the next sequence. The heartbeat sends the
   * document already there, which the daemon keeps running: `running` tells that apart from a stroke
   * that starts the lights over.
   */
  const send = useCallback(
    async (document: Pattern) => {
      // An in-flight undo or shape write can land after the editor closed and its draft was deleted.
      // Never PUT again from an editor that is gone: the daemon would put the pattern back (#491).
      if (!alive.current) return;
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
      const sentAt = performance.now();
      try {
        const answer = await api.putPatternDraft({ pattern: document, epoch, seq: at });
        if (!alive.current) return;
        if (answer.applied !== false && at > clockSeq.current && at > clockFloor.current) {
          clockSeq.current = at;
          const timing = timingKey(document);
          const was = running.current;
          // The same timing (the document again, or other colours): the lights' clock is the one
          // running. Anything else was installed on the lights when the PUT landed.
          const clock =
            was !== null && was.timing === timing ? was.clock : (sentAt + performance.now()) / 2;
          running.current = { timing, clock };
          setLightsClock(clock);
        }
        if (answer.heartbeat_ms > 0) heartbeat.current = answer.heartbeat_ms;
        if (anotherWindow(answer)) setDaemonNote(SUPERSEDED);
        else if (answer.note) setDaemonNote(answer.note);
        // A stroke that landed makes an earlier "another window has the lights" note stale.
        else if (answer.applied === true) setDaemonNote(null);
        setProblem(null);
      } catch (error) {
        if (!alive.current) return;
        setProblem(errorText(error));
      }
    },
    [epoch],
  );

  /**
   * The newest document, for the heartbeat's own PUT: moved with every draft that goes out, so a
   * beat that lands between a stroke and its render sends that stroke and never the colours the
   * render began in.
   */
  const latest = useRef<Pattern>(draftDocument(pattern, 0, false));
  const draft = useCallback(
    (document: Pattern) => {
      latest.current = document;
      void send(document);
    },
    [send],
  );

  // The lights and devices the daemon lists: which light a shape change writes, and the state of
  // that light before an undo tries to write it again (#491).
  const lights = useSnap((state) => state.lights);
  const devices = useSnap((state) => state.devices);

  /**
   * Writes a light's arrangement through the lights route, for a shape change and for its undo or
   * redo (#481). A failure is named in the note row and thrown, so an undo leaves its history step
   * where it was instead of moving the pattern's shape away from the light's.
   */
  const writeShape = useCallback(async (lightId: string, arrangement: Arrangement) => {
    setShapeProblem(null);
    try {
      await api.lights.patch(lightId, { arrangement });
      await refreshLights();
    } catch (error) {
      // The note row is gone with the editor: no state update after the unmount (#491).
      if (alive.current) setShapeProblem(errorText(error));
      throw error;
    }
  }, []);

  /**
   * The light's arrangement as the daemon last listed it. An undo or redo reads this before writing
   * the light: a change made in Devices since the shape change is not overwritten (#491).
   */
  const getShapeLight = useCallback(
    (lightId: string) => lights?.find((l) => l.light.id === lightId)?.light.arrangement,
    [lights],
  );
  const onShapeSkipped = useCallback(
    (_step: ShapeStep, expected: Arrangement, current: Arrangement) => {
      if (!alive.current) return;
      setShapeProblem(
        `the light is a ${current} now, not the ${expected} this change needs; it was changed elsewhere, so the pattern kept its shape`,
      );
    },
    [],
  );

  // Play (#468, #481): whether the lights play the whole timeline. Read by the editor when it
  // drafts, so a stroke made while it plays goes out as the whole timeline and one made otherwise as
  // the selected frame alone.
  const [playing, setPlaying] = useState(false);
  const playingNow = useRef(false);
  const isPlaying = useCallback(() => playingNow.current, []);

  const editor = usePatternEditor(pattern, {
    onDraft: draft,
    onShapeLight: writeShape,
    getShapeLight,
    onShapeSkipped,
    isPlaying,
  });

  // The preview (#468): Play runs the whole document on the dots with the daemon's own maths
  // (`lib/patternPlay.ts`); stopped, the dots show the frame being painted. The lights follow Play:
  // the whole timeline while it plays, the selected frame while it does not (#481).
  const playable = useMemo(
    () => !isStill(timelineOf({ frames: editor.frames, playback: editor.loop ? undefined : { loop: false } })),
    [editor.frames, editor.loop],
  );
  const reduced = motionOffNow();
  const cannotPlay = reduced
    ? "Reduced motion is on: the preview stays on the selected frame"
    : playable
      ? null
      : "Nothing to play: add a frame and give the frames a hold";
  const playingFrames = playing && cannotPlay === null;
  const play = useMemo(
    () => (playingFrames ? { frames: editor.frames, playback: editor.loop ? undefined : { loop: false } } : null),
    [playingFrames, editor.frames, editor.loop],
  );
  // Play or Stop changes what the lights are given: the whole timeline, or the selected frame. The
  // preview waits for the answer to the PUT that starts the lights before it runs, so the two start
  // together; an answer to a PUT sent before the key was pressed may not set its clock.
  const pressed = useRef(false);
  useEffect(() => {
    playingNow.current = playingFrames;
    if (!pressed.current) {
      pressed.current = true;
      return;
    }
    clockFloor.current = seq.current;
    setLightsClock(null);
    editor.refresh();
  }, [playingFrames, editor.refresh]);

  // The shape picker writes the arrangement of the light this pattern was built on (#468). When
  // more than one light is in the pattern's shape, the owner picks which one it writes to (#481);
  // a light of another arrangement is offered only when no light of this size matches the pattern's
  // shape, where the shape change is what brings it into step. The best match is the default, and
  // nothing is patched until a shape is chosen.
  const choosable = useMemo(() => choosableLights(editor.builtOn, lights, devices), [editor.builtOn, lights, devices]);
  const [chosenId, setChosenId] = useState<string | null>(null);
  const light = choosable.find((l) => l.light.id === chosenId) ?? choosable[0] ?? null;
  const zone = zoneOf(light, devices);
  const chooseShape = async (shape: Shape) => {
    setShapeProblem(null);
    // The light keeps its own arrangement, so the step records both ends for the history (#481).
    const step =
      light && light.light.arrangement !== shape
        ? { light: light.light.id, from: light.light.arrangement, to: shape }
        : null;
    if (step) {
      try {
        await writeShape(step.light, step.to);
      } catch {
        // The light did not change, so neither does the pattern's shape: the header says why.
        return;
      }
    }
    // The shape PATCH can land after the editor closed. Do not change the pattern, or schedule a
    // draft, from an editor that is gone (#491).
    if (!alive.current) return;
    editor.setShape(shape, gridOfZone(zone), step);
  };

  // The LED order of the light this pattern plays on (#468). The notch writes `first_led` and the
  // arrow writes `reversed` through the same lights route as the shape. A set that lands identifies
  // the physical LED on the real device, so the owner can see which one is LED 1. These are light
  // properties, not pattern colours: they take effect at once and sit outside the pattern's colour
  // undo history, which is all this editor's history holds.
  // The light's own LED count is the run `first_led` counts in and the daemon checks it against.
  // `choosableLights` only returns lights whose count equals the pattern's, so the two cannot differ
  // while a light is found; the light's count is used so the notch never depends on the picture.
  const lightLeds = light?.led_count ?? Math.max(1, editor.builtOn.led_count);
  const reversed = light?.light.reversed ?? false;
  const firstLed = effectiveFirstLed(light?.light.first_led, reversed, lightLeds);
  // Identify addresses a LED of the zone; `first_led` counts inside the light's slice.
  const identifyFirst = (led: number) => {
    if (!light) return;
    void api
      .identify(light.light.device_id, {
        zone: light.light.zone_index,
        led: identifyLed(light.light.leds, led),
        count: 1,
        stop: false,
      })
      .catch(() => undefined);
  };
  const setFirstLed = async (led: number) => {
    if (!light) return;
    setShapeProblem(null);
    try {
      await api.lights.patch(light.light.id, { first_led: led });
      await refreshLights();
      identifyFirst(led);
    } catch (error) {
      setShapeProblem(errorText(error));
    }
  };
  const setReversed = async (next: boolean) => {
    if (!light) return;
    setShapeProblem(null);
    try {
      await api.lights.patch(
        light.light.id,
        directionPatch(next, light.light.first_led, reversed, lightLeds),
      );
      await refreshLights();
    } catch (error) {
      setShapeProblem(errorText(error));
    }
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
  const { allowBack, dropSentinel } = useLeaveGuard({
    dirty: editor.dirty,
    sentinel: PATTERN_GUARD,
    onAsk: (reason) => setLeaving(reason),
    isLeaving: () => pendingLeave.current,
    strictEntry: true,
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
      setSaveProblem(errorText(error));
    }
    setSaving(false);
    if (landed) then?.();
  };

  /**
   * Carries out the leave the prompt was asked about, as it was asked: a Back the guard caught is
   * not the same leave as the Cancel key pressed afterwards.
   *
   * The guard's entry is a copy of the editor's on top of it. A leave takes it back first, then
   * closes or navigates from the editor's own entry:
   *   - a rail link's page change is a push, which drops the copy; from the editor's own page it
   *     replaces the editor's entry, so one Back goes to the page the person came from, and the
   *     copy stays ahead of the new page, inert;
   *   - a Cancel, or a Back with no page of the app behind, replaces the editor's entry with the
   *     effects page, and the copy stays ahead of it (a browser cannot delete a forward entry);
   *   - a Back with a page behind goes to that page, and both entries stay ahead of it: the copy,
   *     and the editor's own, which is marked on the way past.
   * Both leftovers carry the guard's state key, and `PatternEditorPage` opens no editor for an
   * entry that has it: a Forward onto one lands on the effects page.
   */
  const goAfterLeave = (reason: LeaveReason) => {
    pendingLeave.current = true;
    allowBack();
    if (reason === "back" && openedInApp.current) {
      // Back goes past the guard's entry and past the page itself, one page in all. The editor's
      // own entry stays ahead of the page the person lands on, so it is marked spent on the way.
      dropSentinel(() => {
        window.history.replaceState(
          { ...(window.history.state ?? {}), [PATTERN_GUARD]: PATTERN_ENTRY_SPENT },
          "",
          window.location.href,
        );
        window.history.back();
      });
      return;
    }
    // A rail link the shell held is the page change that goes now. A Back with no page of the
    // app behind the editor closes onto the page the Cancel key goes to, not out of the document.
    if (reason !== "back" && hasPendingNavigation()) {
      dropSentinel(() => continueNavigation({ replace: ownsEntry }));
      return;
    }
    dropSentinel(() => onCancel?.());
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
        {choosable.length > 1 && (
          <Select
            label="Which light the shape change writes to"
            value={light?.light.id ?? ""}
            options={choosable.map((l) => ({ value: l.light.id, label: lightLabel(l, choosable) }))}
            onChange={setChosenId}
          />
        )}
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
          play={play}
          playClock={lightsClock}
          label={`${name}, ${kindWords(editor.builtOn)}`}
          onBegin={editor.beginStroke}
          onStroke={editor.stroke}
          onEnd={editor.endStroke}
          ledOrder={(dots, box) => (
            <div className={s.ledOrderOverlay}>
              <LedOrderControl
                ledCount={lightLeds}
                firstLed={firstLed}
                reversed={reversed}
                ring={editor.builtOn.arrangement === "ring"}
                label={name}
                width={Math.max(1, box.width)}
                height={Math.max(1, box.height)}
                dots={dots}
                onFirstLed={(led) => void setFirstLed(led)}
              />
            </div>
          )}
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
        {/* The two LED-order keys, worded and in the stage's own top band: the notch says where the
            run starts, these say which way it goes and light that LED on the device. */}
        <div className={s.orderKeys} role="group" aria-label="LED order">
          <Chip
            icon={<Crosshair size={14} weight="bold" aria-hidden="true" />}
            label={`Find LED ${firstLed + 1}`}
            title="Light the first LED on the device"
            testId="led-order-identify"
            onClick={() => identifyFirst(firstLed)}
          >
            <span className={s.keyWord}>Find LED {firstLed + 1}</span>
          </Chip>
          <Chip
            icon={<ArrowsLeftRight size={14} weight="bold" aria-hidden="true" />}
            label="Reverse"
            title="Run the LEDs the other way round"
            testId="led-order-direction"
            pressed={reversed}
            onClick={() => void setReversed(!reversed)}
          >
            <span className={s.keyWord}>Reverse</span>
          </Chip>
        </div>
      </div>

      <MotionControl
        motion={editor.motion}
        onMotion={editor.setMotion}
        onBegin={editor.beginMotionDrag}
        onEnd={editor.endMotionDrag}
      />

      <FrameStrip
        builtOn={editor.builtOn}
        frames={editor.frames}
        selected={editor.selected}
        loop={editor.loop}
        fps={editor.fps}
        playing={playingFrames}
        cannotPlay={cannotPlay}
        onSelect={editor.selectFrame}
        onAdd={editor.addFrame}
        onDuplicate={editor.duplicateFrame}
        onDelete={editor.deleteFrame}
        onMove={editor.moveFrame}
        onHold={editor.setHold}
        onTransition={editor.setTransition}
        onLoop={editor.setLoop}
        onFps={editor.applyFps}
        onPlay={() => setPlaying((on) => !on)}
      />

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

/** A chooser row: two lights can share a label (two DRAM sticks), so number the duplicates. */
function lightLabel(view: LightView, all: LightView[]): string {
  const base = view.label || view.light.id;
  const peers = all.filter((l) => (l.label || l.light.id) === base);
  return peers.length > 1 ? `${base} (${peers.indexOf(view) + 1})` : base;
}

/** 16 random hex characters, for one editor's draft epoch. */
function newEpoch(): string {
  const bytes = new Uint8Array(8);
  if (typeof globalThis.crypto?.getRandomValues === "function") globalThis.crypto.getRandomValues(bytes);
  else for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}
