"use client";

// The Effects inspector in edit mode (issue #2, D2.2 to D2.5): the source of an `html:` effect
// beside the daemon's live preview of it, with Save through the import route, a draft on the
// real lights while the editor is open, and the property controls read from the preview's own
// schema. Nothing here writes an effect file except Save, and saving ends the draft.

import { useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState, type Ref } from "react";
import {
  ArrowCounterClockwise,
  CaretLeft,
  Code,
  FloppyDisk,
  Lightbulb,
  Play,
  Warning,
  X,
} from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { LoopPreview } from "@/components/LoopPreview";
import { NameField } from "@/components/NameField";
import { Panel, StateDot } from "@/components/Panel";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { EffectCode } from "@/views/effects/EffectCode";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { cancelNavigation, continueNavigation, setNavigationGuard } from "@/lib/navigationGuard";
import { matchingPreset } from "@/lib/presetMatch";
import { sourceLineOf } from "@/lib/effectSource";
import { refreshAll } from "@/lib/store";
import { thumbFromBase64, type Thumb } from "@/lib/thumbs";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { EffectPreviewResponse } from "@/proto/EffectPreviewResponse";
import type { Preset } from "@/proto/Preset";
import type { PropKind } from "@/proto/PropKind";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { Props } from "@/proto/props";
import type { PropValue } from "@/proto/PropValue";

/** Sent this long after the last keystroke (D2.4). */
const PREVIEW_DEBOUNCE_MS = 400;

/** Used when the daemon's own status leaves the timings out. */
const HEARTBEAT_FALLBACK_MS = 5_000;
const LEASE_FALLBACK_MS = 15_000;

/** What the preview is doing, in the order a keystroke takes it. */
type PreviewState = "idle" | "rendering" | "ok" | "error" | "busy";

/** What the draft on the lights is doing. */
type DraftState = "off" | "starting" | "running" | "expiring" | "restoring" | "restore-error";

export interface EditorTarget {
  /** The effect being edited, or null for a file that has never been saved. */
  id: string | null;
  name: string;
  fileName: string;
  source: string;
  /** The values the effect runs with now; a new file starts empty. */
  props: Props;
  /** The loaded schema, for the presets and the starting values. Null for a new file. */
  schema: PropertySchema | null;
  presets: Preset[];
  userPresets: Preset[];
}

/** The saving end: the saved effect, and why it did not start when that happened. */
export type SavedEffect = { id: string; name: string; startError: string | null };

export type EffectEditorHandle = { requestLeave: () => void };

/**
 * The effect editor. The page keeps the target (what is open) and swaps the inspector for this;
 * the draft, the preview, the save and the unsaved prompt all belong to the editor, because they
 * must end when the editor does. `ref.requestLeave` is how the sheet's Back reaches the prompt.
 */
export function EffectEditor({
  target,
  narrow,
  onClose,
  onSaved,
  ref,
}: {
  target: EditorTarget;
  narrow: boolean;
  onClose: () => void;
  onSaved: (saved: SavedEffect) => void;
  ref?: Ref<EffectEditorHandle>;
}) {
  const [source, setSource] = useState(target.source);
  const [savedSource, setSavedSource] = useState(target.source);
  const [fileName, setFileName] = useState(target.fileName);
  const [values, setValues] = useState<Values>(target.props);
  const [showCode, setShowCode] = useState(!narrow);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  // A new file whose name is taken: the same replace prompt the import uses, said as "file
  // exists" so it is not mistaken for the preset conflict.
  const [clash, setClash] = useState<string | null>(null);
  const [leaving, setLeaving] = useState(false);
  // The line an error row was asked to show, and the line a new error is brought into view on.
  const [focus, setFocus] = useState<{ line: number; nonce: number } | null>(null);
  const [reveal, setReveal] = useState<{ line: number; nonce: number } | null>(null);

  const [preview, setPreview] = useState<EffectPreviewResponse | null>(null);
  // The last render with no errors: its loop, the daemon's hash of the exact source, and the
  // source text itself, so a later failure can say whether the loop still matches the draft.
  const [lastGood, setLastGood] = useState<{ thumb: Thumb; hash: string; source: string } | null>(null);
  const [previewState, setPreviewState] = useState<PreviewState>("idle");
  const [previewError, setPreviewError] = useState<string | null>(null);

  const [draft, setDraft] = useState<DraftStatus | null>(null);
  const [draftState, setDraftState] = useState<DraftState>("off");
  const [draftError, setDraftError] = useState<string | null>(null);
  const [draftNote, setDraftNote] = useState<string | null>(null);
  const [expiresIn, setExpiresIn] = useState(0);

  const dirty = source !== savedSource;
  const isNew = target.id === null;
  const name = preview?.schema.name ?? target.name;
  const schema = preview?.schema ?? target.schema;
  const defs = schema?.properties ?? [];

  // Async work reads these, never the closed-over state.
  const sourceRef = useRef(source);
  const valuesRef = useRef(values);
  const mounted = useRef(true);
  const draftOn = useRef(false);
  const beatTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const heartbeatMs = useRef(HEARTBEAT_FALLBACK_MS);
  const leaseMs = useRef(LEASE_FALLBACK_MS);
  const lastBeatAt = useRef(0);
  const previewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const requestSeq = useRef(0);
  const firstPreview = useRef(true);
  const restoreStarted = useRef(false);
  const revealedLine = useRef<number | null>(null);
  // The browser's Back/Forward and a held in-app page change are two different asks: the first
  // is answered by going back past the guard entry, the second by running the held navigation.
  const backAsked = useRef(false);
  const allowBack = useRef(false);
  const sentinelPushed = useRef(false);
  // A Save whose activate failed and whose draft could not be stopped: the result waits for
  // the Retry that ends the draft before the editor closes.
  const pendingSaved = useRef<SavedEffect | null>(null);
  const previewRef = useRef<(retried?: boolean) => Promise<void>>(async () => {});

  useEffect(() => {
    sourceRef.current = source;
  }, [source]);
  useEffect(() => {
    valuesRef.current = values;
  }, [values]);

  // Unsaved code is never dropped by an in-app page change: the shell asks this guard first,
  // and the same Save / Discard / Keep editing prompt answers it. Registered only while there
  // is something to lose.
  useEffect(() => {
    if (!dirty) {
      setNavigationGuard(null);
      // A guard entry that was never used is taken back, so one Back still leaves the page.
      if (sentinelPushed.current) {
        sentinelPushed.current = false;
        window.history.back();
      }
      return;
    }
    setNavigationGuard(() => {
      backAsked.current = false;
      setLeaving(true);
      return false;
    });
    // The browser's own Back and Forward belong to the router, not the shell. One extra history
    // entry at this page's own URL catches them: Back lands on the same page, this handler puts
    // the entry back, and the prompt asks. Discard then goes back twice: past the guard entry
    // and past the page itself.
    const onPop = () => {
      if (allowBack.current) return;
      window.history.pushState({ canvasRgbEditor: "guard" }, "", window.location.href);
      sentinelPushed.current = true;
      backAsked.current = true;
      setLeaving(true);
    };
    window.history.pushState({ canvasRgbEditor: "guard" }, "", window.location.href);
    sentinelPushed.current = true;
    window.addEventListener("popstate", onPop);
    return () => {
      window.removeEventListener("popstate", onPop);
      setNavigationGuard(null);
    };
  }, [dirty]);

  // Reloading or closing the window is the browser's own navigation: it asks in its words.
  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

  // ---- preview (D2.4) ----------------------------------------------------------------

  const runPreview = useCallback(async (retried = false) => {
    const seq = ++requestSeq.current;
    setPreviewState("rendering");
    setPreviewError(null);
    const sent = sourceRef.current;
    try {
      const answer = await api.previewEffect({
        html: sent,
        props: { ...valuesRef.current },
      });
      if (!mounted.current || seq !== requestSeq.current) return;
      setPreview(answer);
      if (answer.errors.length === 0) {
        const thumb = thumbFromBase64(answer.thumbnail);
        if (thumb) setLastGood({ thumb, hash: answer.source_hash, source: sent });
        setPreviewState("ok");
        revealedLine.current = null;
      } else {
        setPreviewState("error");
        // A new first error is brought into view, so the gutter mark is not a scroll away. The
        // same error on every beat never moves the view the typing is in.
        const first = answer.errors.find((error) => error.line !== undefined);
        if (first?.line !== undefined) {
          const line = sourceLineOf(sent, first.line);
          if (line !== revealedLine.current) {
            revealedLine.current = line;
            setReveal((before) => ({ line, nonce: (before?.nonce ?? 0) + 1 }));
          }
        }
      }
    } catch (e) {
      if (!mounted.current || seq !== requestSeq.current) return;
      if (e instanceof ApiFailure && e.status === 503) {
        setPreviewState("busy");
        // The daemon is rendering two previews: wait what it asks for and try once. A second
        // busy answer leaves the state up with Try again; a busy daemon is never polled.
        if (!retried) {
          retryTimer.current = setTimeout(() => void previewRef.current(true), e.retryAfterMs ?? 1_000);
        }
        return;
      }
      setPreviewState("error");
      setPreviewError(messageOf(e));
    }
  }, []);

  useEffect(() => {
    previewRef.current = runPreview;
  }, [runPreview]);

  const valuesKey = useMemo(
    () => JSON.stringify(Object.keys(values).sort().map((k) => [k, values[k]])),
    [values],
  );

  useEffect(() => {
    if (previewTimer.current) clearTimeout(previewTimer.current);
    const delay = firstPreview.current ? 0 : PREVIEW_DEBOUNCE_MS;
    firstPreview.current = false;
    previewTimer.current = setTimeout(() => void runPreview(), delay);
    return () => {
      if (previewTimer.current) clearTimeout(previewTimer.current);
    };
  }, [source, valuesKey, runPreview]);

  useEffect(
    () => () => {
      mounted.current = false;
      if (previewTimer.current) clearTimeout(previewTimer.current);
      if (retryTimer.current) clearTimeout(retryTimer.current);
      // An editor that goes away without the prompt being answered leaves no held page change.
      setNavigationGuard(null);
      cancelNavigation();
    },
    [],
  );

  const responseThumb = useMemo(
    () => (preview ? thumbFromBase64(preview.thumbnail) : null),
    [preview],
  );
  // A failure that is not about the current source leaves the old loop on screen. Its
  // source_hash is what says which draft it is (D2.4), so it is shown with the stale label.
  const errors = preview?.errors ?? [];
  const warnings = preview?.warnings ?? [];
  const stale = previewState === "error" && lastGood !== null && lastGood.source !== source;
  const thumb = stale ? lastGood?.thumb ?? null : responseThumb ?? lastGood?.thumb ?? null;
  const liveNote =
    previewState === "rendering"
      ? "Rendering…"
      : previewState === "busy"
        ? "Busy: the daemon is rendering other previews"
        : stale
          ? "Stale: last good render"
          : previewState === "error"
            ? errors.length > 0
              ? "Preview stopped: the code has an error"
              : "Preview stopped: the daemon did not answer"
            : "Preview";

  const codeErrors = useMemo(
    () =>
      (preview?.errors ?? []).map((error) => ({
        message: error.message,
        line: error.line === undefined ? undefined : sourceLineOf(source, error.line),
        column: error.column,
      })),
    [preview, source],
  );
  // ---- draft on the lights (D2.5) ----------------------------------------------------

  const beatRef = useRef<() => Promise<void>>(async () => {});

  const scheduleBeat = useCallback((ms: number) => {
    if (beatTimer.current) clearTimeout(beatTimer.current);
    beatTimer.current = setTimeout(() => void beatRef.current(), ms);
  }, []);

  const applyStatus = useCallback((status: DraftStatus) => {
    setDraft(status);
    heartbeatMs.current = status.heartbeat_ms || HEARTBEAT_FALLBACK_MS;
    leaseMs.current = status.lease_ms || LEASE_FALLBACK_MS;
    lastBeatAt.current = Date.now();
  }, []);

  const beat = useCallback(async () => {
    if (!draftOn.current) return;
    try {
      const status = await api.putDraft({
        html: sourceRef.current,
        props: { ...valuesRef.current },
      });
      if (!mounted.current || !draftOn.current) return;
      applyStatus(status);
      setDraftState("running");
      setDraftError(null);
      scheduleBeat(status.heartbeat_ms || HEARTBEAT_FALLBACK_MS);
    } catch (e) {
      if (!mounted.current || !draftOn.current) return;
      // One missed beat is survivable: the lease has three. The banner counts what is left.
      setDraftError(messageOf(e));
      setDraftState("expiring");
      scheduleBeat(heartbeatMs.current);
    }
  }, [applyStatus, scheduleBeat]);

  useEffect(() => {
    beatRef.current = beat;
  }, [beat]);

  const startDraft = async () => {
    setDraftState("starting");
    setDraftError(null);
    setDraftNote(null);
    restoreStarted.current = false;
    try {
      const status = await api.putDraft({
        html: sourceRef.current,
        props: { ...valuesRef.current },
      });
      if (!mounted.current) return;
      draftOn.current = true;
      applyStatus(status);
      setDraftState("running");
      scheduleBeat(status.heartbeat_ms || HEARTBEAT_FALLBACK_MS);
    } catch (e) {
      if (!mounted.current) return;
      setDraftState("off");
      setDraftError(messageOf(e));
    }
  };

  const stopDraft = async () => {
    draftOn.current = false;
    if (beatTimer.current) clearTimeout(beatTimer.current);
    beatTimer.current = null;
    setDraftState("restoring");
    setDraftError(null);
    try {
      const status = await api.deleteDraft();
      if (!mounted.current) return;
      setDraft(null);
      setDraftState("off");
      if (status.note) setDraftNote(status.note);
      // A Save that was waiting for this stop may now finish and close.
      const saved = pendingSaved.current;
      if (saved) {
        pendingSaved.current = null;
        goAfterLeave();
        onSaved(saved);
      }
    } catch (e) {
      if (!mounted.current) return;
      setDraftState("restore-error");
      setDraftError(messageOf(e));
    }
  };

  // A reload during a draft opens with the banner: the daemon still holds the slot for a few
  // seconds, and this editor takes the lease over with its next beat.
  useEffect(() => {
    let stop = false;
    api
      .draft()
      .then((status) => {
        if (stop || !mounted.current || !status.active) return;
        draftOn.current = true;
        applyStatus(status);
        setDraftState("running");
        scheduleBeat(status.heartbeat_ms || HEARTBEAT_FALLBACK_MS);
      })
      .catch(() => {
        // The banner is a nicety: a read that fails leaves it off, and Try on the lights still
        // works. Nothing about the editor depends on it.
      });
    return () => {
      stop = true;
    };
  }, [applyStatus, scheduleBeat]);

  // Leaving the editor stops the draft, whether or not Save was pressed.
  useEffect(
    () => () => {
      if (draftOn.current) {
        draftOn.current = false;
        if (beatTimer.current) clearTimeout(beatTimer.current);
        void api.deleteDraft().catch(() => undefined);
      }
    },
    [],
  );

  // While the draft runs (or while its lease runs out), the banner counts the seconds left.
  // When the count runs out the daemon restores the previous effect by itself; the editor waits
  // for the slot to be gone and says so. The sweeper runs once a second.
  useEffect(() => {
    if (draftState !== "expiring") return;
    const tick = () => {
      const left = Math.max(0, lastBeatAt.current + leaseMs.current - Date.now());
      setExpiresIn(Math.ceil(left / 1000));
      if (left > 0 || restoreStarted.current) return;
      restoreStarted.current = true;
      setDraftState("restoring");
      let tries = 0;
      const ask = async () => {
        tries += 1;
        try {
          const status = await api.draft();
          if (!mounted.current) return;
          if (!status.active) {
            draftOn.current = false;
            setDraft(null);
            setDraftState("off");
            if (status.note) setDraftNote(status.note);
            return;
          }
        } catch {
          // A read that fails is not an ending: the next one may say it is gone.
        }
        if (tries >= 5) {
          setDraftState("restore-error");
          setDraftError("the draft is still on the lights; its ending could not be seen");
          restoreStarted.current = false;
          return;
        }
        setTimeout(() => void ask(), 1_000);
      };
      void ask();
    };
    tick();
    const timer = setInterval(tick, 1_000);
    return () => clearInterval(timer);
  }, [draftState]);

  const tryPreset = (preset: Preset) => {
    const next: Values = {};
    for (const def of defs) next[def.key] = preset.props[def.key] ?? defaultOf(def.kind);
    setValues(next);
    // A chip inside the editor acts on the draft, never on the saved effect (D2.2).
    if (draftOn.current) {
      void api
        .putDraft({ html: sourceRef.current, props: next })
        .then((status) => {
          if (!mounted.current) return;
          applyStatus(status);
          setDraftState("running");
          setDraftError(null);
        })
        .catch((e: unknown) => {
          if (!mounted.current) return;
          setDraftError(messageOf(e));
          setDraftState("expiring");
        });
    }
  };

  // ---- save, revert, leave -----------------------------------------------------------

  /**
   * Carries out the page change the prompt was holding, if any: a browser Back is answered by
   * going back past the guard entry and the page, an in-app link by the navigation it held.
   */
  const goAfterLeave = () => {
    if (backAsked.current) {
      backAsked.current = false;
      allowBack.current = true;
      window.history.go(-2);
    } else {
      continueNavigation();
    }
  };

  const save = async (replace = false) => {
    if (saving) return;
    setSaveError(null);
    const wanted = fileName.trim();
    // An effect that is already on disk is saved over itself. A new file is written without
    // asking to replace: the route answers a clash when another window got the name first,
    // and only then does the prompt ask, and the write retry with replace.
    const replaceOnDisk = !isNew || replace;
    setSaving(true);
    // A Save whose activate fails must not leave a draft running behind a cleared banner.
    const replacingDraft = draftOn.current;
    let savedId: string | null = null;
    let savedName = name;
    try {
      const answer = await api.importEffect({ file_name: wanted, html: source, replace: replaceOnDisk });
      savedId = answer.effect.id;
      savedName = answer.effect.name;
      setSavedSource(source);
      // The activate ends the draft on the lights without putting the old effect back; the
      // draft's own end must not be sent again.
      draftOn.current = false;
      if (beatTimer.current) clearTimeout(beatTimer.current);
      setDraftState("off");
      await refreshAll();
    } catch (e) {
      setSaving(false);
      if (!replaceOnDisk && e instanceof ApiFailure && e.status === 409) {
        setClash(wanted);
        return;
      }
      setSaveError(messageOf(e));
      setClash(null);
      return;
    }
    let startError: string | null = null;
    try {
      await api.activateEffect(savedId, { props: {}, preset: null });
      await refreshAll();
    } catch (e) {
      startError = messageOf(e);
      setSaveError(`Saved, but it did not start: ${startError}`);
      // The saved file did not start, so the draft it would have replaced is stopped too: the
      // lights go back to what they were before it, and the screen says what happened. A stop
      // that fails leaves the banner in its restore-error state with Retry; the editor stays
      // open so the lights and the screen still agree.
      if (replacingDraft) {
        try {
          await api.deleteDraft();
        } catch (stopError) {
          draftOn.current = true;
          setDraftState("restore-error");
          setDraftError(messageOf(stopError));
          pendingSaved.current = { id: savedId, name: savedName, startError };
          setSaving(false);
          setClash(null);
          return;
        }
      }
    }
    setSaving(false);
    setClash(null);
    // A page change that waited on this Save may go now.
    goAfterLeave();
    onSaved({ id: savedId, name: savedName, startError });
  };

  const revert = () => {
    setSource(savedSource);
    setSaveError(null);
  };

  const askLeave = () => {
    if (dirty) setLeaving(true);
    else onClose();
  };

  // The page reaches the prompt through a ref: the sheet's Back and the editor's Done ask the
  // same question, and neither leaves silently (D2.2).
  useImperativeHandle(
    ref,
    () => ({
      requestLeave: () => {
        if (dirty) setLeaving(true);
        else onClose();
      },
    }),
    [dirty, onClose],
  );

  const focusError = (line: number | undefined, index: number) => {
    setShowCode(true);
    setFocus({ line: line ?? 1, nonce: index + 1 + (focus?.nonce ?? 0) });
  };

  const lineCount = source.split("\n").length;
  const running = draftState === "running" || draftState === "expiring" || draftState === "starting";
  // A clean effect still has something to save while a draft runs: the draft's own ending.
  const canSave = !saving && (isNew || dirty || running);
  const presetPressed = (preset: Preset) => {
    if (defs.length === 0) return false;
    return matchingPreset(defs, [preset], { ...target.props, ...values }) !== null;
  };

  const codePane = (
    <div className="eff-code" data-testid="code-panel">
      <div className="eff-code-head">
        {narrow && (
          <IconButton
            label="Back to the effect"
            icon={<CaretLeft size={18} />}
            onClick={() => setShowCode(false)}
          />
        )}
        {!narrow && <Code size={15} aria-hidden="true" />}
        <span className="eff-code-title">Code</span>
        <span className="eff-code-count fig">{lineCount} lines</span>
        <div className="eff-code-actions">
          <Button
            size="sm"
            variant="ghost"
            icon={<ArrowCounterClockwise size={14} />}
            disabled={!dirty || saving}
            onClick={revert}
          >
            Revert
          </Button>
          {narrow && (
            // The banner's Save and the prompt's Save are the primary ones while they show.
            <Button
              size="sm"
              variant={leaving || running ? "default" : "primary"}
              disabled={!canSave}
              onClick={() => void save()}
            >
              Save
            </Button>
          )}
        </div>
      </div>
      <div className="eff-code-body">
        <EffectCode
          value={source}
          onChange={setSource}
          errors={codeErrors}
          focusLine={focus?.line}
          focusNonce={focus?.nonce}
          revealLine={reveal?.line}
          revealNonce={reveal?.nonce}
          label={`The code of ${name}`}
        />
      </div>
      {errors.length > 0 && (
        <ul className="eff-errors" aria-label="Errors in the effect">
          {errors.map((error, i) => (
            <li key={`${error.message}-${i}`}>
              <button
                type="button"
                className="eff-error-row"
                onClick={() => focusError(codeErrors[i]?.line, i)}
              >
                <Warning size={14} aria-hidden="true" />
                <span className="fig eff-error-line">
                  {codeErrors[i]?.line !== undefined ? `line ${codeErrors[i]?.line}` : "error"}
                </span>
                <span className="eff-error-text">{error.message}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {previewError && <p className="error-line eff-preview-error">{previewError}</p>}
    </div>
  );

  // The prompt sits with the actions, or over the phone's code layer when that is what is open:
  // a Back while the code layer is up must not ask a question behind it.
  const leavePrompt = (
    <div className="eff-leave" role="alertdialog" aria-label="Unsaved changes">
      <span>Save your changes before leaving?</span>
      <div className="row-wrap">
        <Button size="sm" variant="primary" disabled={saving} onClick={() => void save()}>
          Save
        </Button>
        <Button
          size="sm"
          variant="danger"
          disabled={saving}
          onClick={() => {
            setLeaving(false);
            // A page change that waited on the prompt goes now.
            goAfterLeave();
            onClose();
          }}
        >
          Discard
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            setLeaving(false);
            backAsked.current = false;
            cancelNavigation();
          }}
        >
          Keep editing
        </Button>
      </div>
    </div>
  );

  return (
    <div
      className="eff-editor"
      data-testid="effect-editor"
      data-narrow={narrow ? "true" : "false"}
      data-leaving={leaving ? "true" : "false"}
    >
      {draftState !== "off" && (
        <div className="eff-draft" data-state={draftState} data-draft={draft?.id ?? ""} role="status">
          <span className="eff-draft-mark" data-state={draftState} aria-hidden="true" />
          <span className="eff-draft-text">
            {draftState === "starting"
              ? "Starting the draft…"
              : draftState === "running"
                ? "Draft on the lights."
                : draftState === "expiring"
                  ? `Draft on the lights, expiring in ${expiresIn}s.`
                  : draftState === "restoring"
                    ? "Restoring the previous effect…"
                    : "The previous effect could not be restored."}
          </span>
          <span className="row-wrap eff-draft-actions">
            {draftState === "restore-error" ? (
              <Button size="sm" icon={<Play size={14} />} onClick={() => void stopDraft()}>
                Retry
              </Button>
            ) : (
              <>
                <Button size="sm" variant="ghost" disabled={draftState === "restoring"} onClick={() => void stopDraft()}>
                  Stop
                </Button>
                {/* One primary action at a time: the prompt's Save is the primary while it asks. */}
                <Button size="sm" variant={leaving ? "default" : "primary"} disabled={saving} onClick={() => void save()}>
                  Save
                </Button>
              </>
            )}
          </span>
        </div>
      )}
      {draftNote && (
        <p className="eff-note" role="status">
          <Warning size={14} aria-hidden="true" /> {draftNote}
        </p>
      )}
      {draftState === "restore-error" && draftError && (
        <p className="error-line" role="alert">
          {draftError}
        </p>
      )}

      <div className="eff-edit-main" data-code={showCode && !narrow ? "true" : "false"}>
        <div className="eff-edit-side">
          <div className="eff-live" data-preview={previewState}>
            <LoopPreview thumb={thumb} label={`Preview of ${name}`} />
            <span
              className="eff-live-note"
              data-tone={previewState === "error" ? "warn" : undefined}
              data-source={stale ? lastGood?.hash : undefined}
              title={stale && lastGood ? `The last good render is from source ${lastGood.hash}` : undefined}
            >
              {liveNote}
            </span>
            {warnings.length > 0 && (
              <ul className="eff-live-warnings" aria-label="Warnings">
                {warnings.map((warning) => (
                  <li key={warning}>{warning}</li>
                ))}
              </ul>
            )}
            {(previewState === "busy" || (previewState === "error" && previewError)) && (
              <Button size="sm" variant="ghost" onClick={() => void runPreview()}>
                Try again
              </Button>
            )}
          </div>

          <div className="eff-edit-title">
            <div className="eff-name-row">
              <h2 className="effect-title" title={name}>
                {name}
              </h2>
              {isNew && <Chip html>New HTML effect</Chip>}
              {!isNew && <StateDot tone={running ? "work" : "mute"}>{running ? "Draft" : "Editing"}</StateDot>}
            </div>
            {isNew ? (
              <NameField
                value={fileName}
                label="File name"
                placeholder="my-effect.html"
                maxLength={120}
                autoFocus={false}
                invalid={saveError !== null}
                onChange={(v) => {
                  setFileName(v);
                  setSaveError(null);
                  setClash(null);
                }}
                onEnter={() => void save()}
                onEscape={() => setLeaving(true)}
              />
            ) : (
              <span className="fig eff-file" title={`Save writes ${fileName}`}>
                {fileName}
              </span>
            )}
            <div className="row-wrap eff-edit-actions">
              {(draftState === "off" || draftState === "starting") && (
                <Button
                  size="sm"
                  icon={<Lightbulb size={15} />}
                  disabled={draftState === "starting"}
                  onClick={() => void startDraft()}
                >
                  {draftState === "starting" ? "Starting…" : "Try on the lights"}
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                icon={<Code size={15} />}
                aria-pressed={showCode}
                onClick={() => setShowCode((v) => !v)}
              >
                {showCode ? "Hide code" : "Show code"}
              </Button>
              {/* Not a second primary beside the banner's Save, and not one while the prompt
                  asks: the prompt's Save is the one to press. */}
              <Button
                size="sm"
                variant={leaving || running ? "default" : "primary"}
                icon={<FloppyDisk size={15} />}
                disabled={!canSave}
                onClick={() => void save()}
              >
                {saving ? "Saving…" : "Save"}
              </Button>
              {!narrow && (
                <Button size="sm" variant="ghost" icon={<X size={15} />} onClick={askLeave}>
                  Done
                </Button>
              )}
            </div>
            {draftState === "off" && draftError && (
              <p className="error-line" role="alert">
                {draftError}
              </p>
            )}
            {saveError && (
              <p className="error-line" role="alert">
                {saveError}
              </p>
            )}
            {clash && (
              <div className="import-note" data-tone="ask" role="alert">
                <span>
                  <span className="eff-clash-tag">file exists</span> Replace <span className="fig">{clash}</span>?
                </span>
                <div className="row-wrap">
                  <Button size="sm" variant="primary" disabled={saving} onClick={() => void save(true)}>
                    Replace
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => setClash(null)}>
                    Cancel
                  </Button>
                </div>
              </div>
            )}
            {leaving && !(narrow && showCode) && leavePrompt}
          </div>

          <div className="eff-side-panels">
            {(target.presets.length > 0 || target.userPresets.length > 0) && (
              <Panel title="Presets" className="eff-presets">
                <div className="preset-chips" role="group" aria-label="Presets">
                  {target.presets.map((preset) => (
                    <Chip
                      key={preset.name}
                      pressed={presetPressed(preset)}
                      label={`Try ${preset.name} on the draft`}
                      onClick={() => tryPreset(preset)}
                    >
                      {preset.name}
                    </Chip>
                  ))}
                  {target.userPresets.map((preset) => {
                    const shadowed = target.presets.some(
                      (builtIn) => builtIn.name.toLowerCase() === preset.name.toLowerCase(),
                    );
                    return (
                      <Chip
                        key={preset.name}
                        pressed={presetPressed(preset)}
                        label={`Try ${preset.name} on the draft${shadowed ? " (shadowed)" : ""}`}
                        title={shadowed ? "A built-in preset now has this name; the built-in wins" : undefined}
                        onClick={() => tryPreset(preset)}
                      >
                        {preset.name}
                        {shadowed && <span className="preset-shadowed">shadowed</span>}
                      </Chip>
                    );
                  })}
                </div>
                <p className="eff-tools-note">A preset sets the values here; it never starts the saved effect.</p>
              </Panel>
            )}

            <Panel title="Properties" className="eff-props">
              {!schema || defs.length === 0 ? (
                <p className="dim eff-empty-props">
                  {previewState === "rendering" || previewState === "idle"
                    ? "The controls appear with the first preview."
                    : "This effect declares no properties."}
                </p>
              ) : (
                <PropertyControls
                  schema={schema}
                  values={values}
                  onChange={(key: string, value: PropValue) => setValues((prev) => ({ ...prev, [key]: value }))}
                />
              )}
            </Panel>
          </div>
        </div>

        {showCode && !narrow && codePane}
      </div>

      {showCode && narrow && (
        <div className="eff-layer" role="dialog" aria-label="The effect's code">
          {leaving && <div className="eff-layer-leave">{leavePrompt}</div>}
          <div className="eff-layer-body">{codePane}</div>
        </div>
      )}
    </div>
  );
}

/** The value a kind starts at, for a preset that leaves a key out. */
function defaultOf(kind: PropKind): PropValue {
  switch (kind.type) {
    case "color":
      return { type: "color", value: kind.default };
    case "number":
      return { type: "number", value: kind.default };
    case "bool":
      return { type: "bool", value: kind.default };
    case "choice":
      return { type: "choice", value: kind.default };
    case "hue":
      return { type: "hue", value: kind.default };
  }
}
